//! Merging every node's `subscribeRepos` stream into one.
//!
//! Each node numbers its own events from 1, so the streams cannot simply be
//! concatenated: a subscriber would see the sequence jump around and its cursor
//! would be meaningless. The gateway therefore assigns its own monotonic `seq`
//! to every frame and rewrites that one field, leaving the signed commit blocks
//! untouched.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use ciborium::Value;
use futures::StreamExt;
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::config::{Config, NodeConfig};
use crate::registry::store::Store;

const NSID: &str = "com.atproto.sync.subscribeRepos";

#[derive(Debug, Clone)]
pub struct Frame {
    pub seq: i64,
    pub bytes: Bytes,
}

/// Splits a frame into its header and body, which are two concatenated
/// DAG-CBOR items in one binary message.
fn split(frame: &[u8]) -> Option<(usize, Value)> {
    let mut cursor = std::io::Cursor::new(frame);
    let _header: Value = ciborium::de::from_reader(&mut cursor).ok()?;
    let header_len = cursor.position() as usize;
    let body: Value = ciborium::de::from_reader(&mut cursor).ok()?;
    Some((header_len, body))
}

pub fn read_seq(frame: &[u8]) -> Option<i64> {
    let (_, body) = split(frame)?;
    let Value::Map(entries) = body else {
        return None;
    };
    entries.iter().find_map(|(k, v)| {
        (k.as_text() == Some("seq"))
            .then(|| v.as_integer())
            .flatten()
            .and_then(|i| i128::from(i).try_into().ok())
    })
}

/// Replaces `seq` in a frame's body, keeping the header bytes verbatim and the
/// remaining body keys in their original order.
pub fn rewrite_seq(frame: &[u8], seq: i64) -> Option<Bytes> {
    let (header_len, body) = split(frame)?;
    let Value::Map(mut entries) = body else {
        return None;
    };

    let mut found = false;
    for (key, value) in entries.iter_mut() {
        if key.as_text() == Some("seq") {
            *value = Value::Integer(seq.into());
            found = true;
        }
    }
    if !found {
        return None;
    }

    let mut out = Vec::with_capacity(frame.len() + 8);
    out.extend_from_slice(&frame[..header_len]);
    ciborium::ser::into_writer(&Value::Map(entries), &mut out).ok()?;
    Some(Bytes::from(out))
}

/// An `#info` frame telling a subscriber its cursor was too old to honour.
pub fn outdated_cursor_frame() -> Bytes {
    let header = Value::Map(vec![
        (Value::Text("op".into()), Value::Integer(1.into())),
        (Value::Text("t".into()), Value::Text("#info".into())),
    ]);
    let body = Value::Map(vec![
        (
            Value::Text("name".into()),
            Value::Text("OutdatedCursor".into()),
        ),
        (
            Value::Text("message".into()),
            Value::Text("requested cursor is older than the gateway's replay buffer".into()),
        ),
    ]);

    let mut out = Vec::new();
    let _ = ciborium::ser::into_writer(&header, &mut out);
    let _ = ciborium::ser::into_writer(&body, &mut out);
    Bytes::from(out)
}

pub struct Firehose {
    config: Arc<Config>,
    store: Arc<Store>,
    next_seq: AtomicI64,
    buffer: RwLock<VecDeque<Frame>>,
    live: broadcast::Sender<Frame>,
    persist: tokio::sync::mpsc::UnboundedSender<Frame>,
    persist_rx: parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Frame>>>,
}

impl Firehose {
    pub async fn new(config: Arc<Config>, store: Arc<Store>) -> anyhow::Result<Arc<Self>> {
        let next_seq = store.firehose_next_seq().await.unwrap_or(1).max(1);
        let (live, _) = broadcast::channel(config.firehose.subscriber_queue.max(16));
        let (persist, persist_rx) = tokio::sync::mpsc::unbounded_channel();

        // Warm the in-memory buffer from disk so a restart can still serve the
        // cursors subscribers already hold.
        let mut buffer = VecDeque::new();
        let keep = config.firehose.replay_buffer as i64;
        if let Ok(frames) = store.frames_after((next_seq - 1 - keep).max(0), keep).await {
            for (seq, bytes) in frames {
                buffer.push_back(Frame {
                    seq,
                    bytes: Bytes::from(bytes),
                });
            }
        }

        tracing::info!(
            next_seq,
            replay_warm = buffer.len(),
            "firehose sequence resumed"
        );
        Ok(Arc::new(Self {
            config,
            store,
            next_seq: AtomicI64::new(next_seq),
            buffer: RwLock::new(buffer),
            live,
            persist,
            persist_rx: parking_lot::Mutex::new(Some(persist_rx)),
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Frame> {
        crate::metrics::FIREHOSE_SUBSCRIBERS.incr();
        self.live.subscribe()
    }

    pub fn current_seq(&self) -> i64 {
        self.next_seq.load(Ordering::Relaxed) - 1
    }

    /// Buffered frames after `cursor`, and whether the cursor was too old.
    pub async fn replay(&self, cursor: Option<i64>) -> (Vec<Frame>, bool) {
        let Some(cursor) = cursor else {
            return (Vec::new(), false);
        };

        let (buffered, oldest_in_memory) = {
            let buffer = self.buffer.read();
            let frames: Vec<Frame> = buffer.iter().filter(|f| f.seq > cursor).cloned().collect();
            (frames, buffer.front().map(|f| f.seq))
        };

        // The buffer covers the request when it starts at or before the frame
        // after the cursor; otherwise fall back to the durable log.
        if oldest_in_memory.is_some_and(|oldest| oldest <= cursor + 1) {
            return (buffered, false);
        }

        let limit = self.config.firehose.replay_buffer as i64;
        match self.store.frames_after(cursor, limit).await {
            Ok(rows) if !rows.is_empty() => {
                let frames: Vec<Frame> = rows
                    .into_iter()
                    .map(|(seq, bytes)| Frame {
                        seq,
                        bytes: Bytes::from(bytes),
                    })
                    .collect();
                let oldest = frames.first().map(|f| f.seq).unwrap_or(cursor + 1);
                (frames, oldest > cursor + 1)
            }
            Ok(_) => {
                let oldest = self.store.oldest_frame_seq().await.unwrap_or(None);
                let outdated = oldest.is_some_and(|o| o > cursor + 1)
                    || oldest_in_memory.is_some_and(|o| o > cursor + 1);
                (buffered, outdated)
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not read the durable replay log");
                (buffered, oldest_in_memory.is_some_and(|o| o > cursor + 1))
            }
        }
    }

    fn publish(&self, frame: Frame) {
        {
            let mut buffer = self.buffer.write();
            buffer.push_back(frame.clone());
            while buffer.len() > self.config.firehose.replay_buffer {
                buffer.pop_front();
            }
        }
        crate::metrics::FIREHOSE_FRAMES.incr();
        // A send error means nobody is subscribed, which is not a problem.
        let _ = self.persist.send(frame.clone());
        let _ = self.live.send(frame);
    }

    /// Batches frames to disk, so a busy firehose is not one write per event.
    async fn persist_frames(self: Arc<Self>) {
        let Some(mut rx) = self.persist_rx.lock().take() else {
            return;
        };

        let keep = self.config.firehose.replay_buffer as i64;
        let mut batch: Vec<(i64, Vec<u8>)> = Vec::with_capacity(256);
        let mut since_prune = 0i64;

        loop {
            let first = match rx.recv().await {
                Some(frame) => frame,
                None => break,
            };
            batch.push((first.seq, first.bytes.to_vec()));

            // Drain whatever else is already queued, up to a bounded batch.
            while batch.len() < 256 {
                match rx.try_recv() {
                    Ok(frame) => batch.push((frame.seq, frame.bytes.to_vec())),
                    Err(_) => break,
                }
            }

            if let Err(e) = self.store.append_frames(&batch).await {
                tracing::warn!(error = %e, frames = batch.len(), "could not persist firehose frames");
            }
            since_prune += batch.len() as i64;
            batch.clear();

            // Prune on the same cadence rather than on a timer, so an idle
            // firehose does no work at all.
            if since_prune >= keep.max(1) {
                since_prune = 0;
                if let Err(e) = self.store.prune_frames(keep).await {
                    tracing::warn!(error = %e, "could not prune the replay log");
                }
            }
        }
    }

    /// Reads one node's stream forever, reconnecting with backoff.
    async fn follow(self: Arc<Self>, node: NodeConfig) {
        let mut backoff = self.config.firehose.reconnect_min_backoff.get();

        loop {
            let cursor = self.store.firehose_cursor(&node.name).await.unwrap_or(None);

            let query = cursor.map(|c| format!("cursor={c}")).unwrap_or_default();
            let url = crate::proxy::ws::upstream_url(&node, NSID, &query);

            match tokio_tungstenite::connect_async(&url).await {
                Ok((stream, _)) => {
                    tracing::info!(node = %node.name, cursor = ?cursor, "firehose connected");
                    backoff = self.config.firehose.reconnect_min_backoff.get();
                    self.clone().pump(&node, stream).await;
                    tracing::warn!(node = %node.name, "firehose disconnected");
                }
                Err(e) => {
                    tracing::warn!(node = %node.name, error = %e, "firehose connect failed");
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(self.config.firehose.reconnect_max_backoff.get());
        }
    }

    async fn pump<S>(self: Arc<Self>, node: &NodeConfig, stream: S)
    where
        S: futures::Stream<Item = tokio_tungstenite::tungstenite::Result<WsMessage>> + Unpin,
    {
        let mut stream = stream;
        let mut last_persist = std::time::Instant::now();
        let mut pending_cursor: Option<i64> = None;

        while let Some(message) = stream.next().await {
            let bytes = match message {
                Ok(WsMessage::Binary(bytes)) => bytes,
                Ok(WsMessage::Close(_)) => break,
                Ok(_) => continue,
                Err(e) => {
                    tracing::debug!(node = %node.name, error = %e, "firehose stream error");
                    break;
                }
            };

            let upstream_seq = read_seq(&bytes);
            let gateway_seq = self.next_seq.fetch_add(1, Ordering::Relaxed);

            let rewritten = match rewrite_seq(&bytes, gateway_seq) {
                Some(rewritten) => rewritten,
                None => {
                    // Frames without a `seq`, such as `#info`, pass through and
                    // do not consume a sequence number.
                    self.next_seq.fetch_sub(1, Ordering::Relaxed);
                    Bytes::from(bytes.to_vec())
                }
            };

            self.publish(Frame {
                seq: gateway_seq,
                bytes: rewritten,
            });

            if let Some(seq) = upstream_seq {
                pending_cursor = Some(seq);
            }

            // Persist at most once a second: these nodes often run from SD cards.
            if last_persist.elapsed() >= Duration::from_secs(1)
                && let Some(seq) = pending_cursor.take()
            {
                self.persist(&node.name, seq).await;
                last_persist = std::time::Instant::now();
            }
        }

        if let Some(seq) = pending_cursor {
            self.persist(&node.name, seq).await;
        }
    }

    async fn persist(&self, node: &str, upstream_seq: i64) {
        if let Err(e) = self
            .store
            .put_firehose_cursor(node, upstream_seq, self.next_seq.load(Ordering::Relaxed))
            .await
        {
            tracing::warn!(node, error = %e, "could not persist firehose cursor");
        }
    }

    pub fn spawn(self: Arc<Self>) -> Vec<tokio::task::JoinHandle<()>> {
        let mut tasks: Vec<_> = self
            .config
            .nodes
            .iter()
            .cloned()
            .map(|node| tokio::spawn(self.clone().follow(node)))
            .collect();
        tasks.push(tokio::spawn(self.clone().persist_frames()));
        tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: i64, extra: bool) -> Vec<u8> {
        let header = Value::Map(vec![
            (Value::Text("op".into()), Value::Integer(1.into())),
            (Value::Text("t".into()), Value::Text("#commit".into())),
        ]);

        let mut body = vec![
            (Value::Text("seq".into()), Value::Integer(seq.into())),
            (
                Value::Text("repo".into()),
                Value::Text("did:plc:alice".into()),
            ),
        ];
        if extra {
            body.push((Value::Text("blocks".into()), Value::Bytes(vec![1, 2, 3])));
            body.push((
                Value::Text("commit".into()),
                Value::Tag(42, Box::new(Value::Bytes(vec![0, 1, 2]))),
            ));
        }

        let mut out = Vec::new();
        ciborium::ser::into_writer(&header, &mut out).unwrap();
        ciborium::ser::into_writer(&Value::Map(body), &mut out).unwrap();
        out
    }

    #[test]
    fn reads_the_sequence_out_of_a_frame() {
        assert_eq!(read_seq(&frame(1234, true)), Some(1234));
        assert_eq!(read_seq(b"not cbor at all"), None);
    }

    #[test]
    fn rewrites_the_sequence_and_keeps_everything_else() {
        let original = frame(7, true);
        let rewritten = rewrite_seq(&original, 9999).unwrap();

        assert_eq!(read_seq(&rewritten), Some(9999));

        // The header is copied verbatim.
        let (header_len, _) = split(&original).unwrap();
        assert_eq!(&rewritten[..header_len], &original[..header_len]);

        // Body keys keep their order, and the CID tag survives.
        let (_, body) = split(&rewritten).unwrap();
        let Value::Map(entries) = body else {
            panic!("body should be a map");
        };
        let keys: Vec<_> = entries.iter().filter_map(|(k, _)| k.as_text()).collect();
        assert_eq!(keys, vec!["seq", "repo", "blocks", "commit"]);
        assert!(matches!(entries[3].1, Value::Tag(42, _)));
    }

    #[test]
    fn a_frame_without_a_sequence_is_left_alone() {
        let header = Value::Map(vec![
            (Value::Text("op".into()), Value::Integer(1.into())),
            (Value::Text("t".into()), Value::Text("#info".into())),
        ]);
        let body = Value::Map(vec![(
            Value::Text("name".into()),
            Value::Text("OutdatedCursor".into()),
        )]);

        let mut raw = Vec::new();
        ciborium::ser::into_writer(&header, &mut raw).unwrap();
        ciborium::ser::into_writer(&body, &mut raw).unwrap();

        assert!(read_seq(&raw).is_none());
        assert!(rewrite_seq(&raw, 5).is_none());
    }

    #[test]
    fn the_outdated_cursor_frame_is_well_formed() {
        let raw = outdated_cursor_frame();
        let (_, body) = split(&raw).unwrap();
        let Value::Map(entries) = body else {
            panic!("body should be a map");
        };
        assert_eq!(entries[0].1.as_text(), Some("OutdatedCursor"));
    }

    fn config(replay_buffer: usize) -> Arc<Config> {
        Arc::new(Config {
            firehose: crate::config::FirehoseConfig {
                replay_buffer,
                ..Default::default()
            },
            ..Config::default()
        })
    }

    async fn firehose(replay_buffer: usize) -> Arc<Firehose> {
        let store = Arc::new(Store::open_in_memory().await.unwrap());
        Firehose::new(config(replay_buffer), store).await.unwrap()
    }

    #[tokio::test]
    async fn starts_its_sequence_at_one() {
        let firehose = firehose(8).await;
        assert_eq!(firehose.current_seq(), 0);
    }

    #[tokio::test]
    async fn replays_only_frames_after_the_cursor() {
        let firehose = firehose(8).await;
        for seq in 1..=5 {
            firehose.publish(Frame {
                seq,
                bytes: Bytes::from(frame(seq, false)),
            });
        }

        let (frames, outdated) = firehose.replay(Some(3)).await;
        assert!(!outdated);
        assert_eq!(frames.iter().map(|f| f.seq).collect::<Vec<_>>(), vec![4, 5]);

        // No cursor means live-only, no replay.
        let (frames, _) = firehose.replay(None).await;
        assert!(frames.is_empty());
    }

    #[tokio::test]
    async fn flags_a_cursor_older_than_the_buffer() {
        let firehose = firehose(3).await;
        for seq in 1..=6 {
            firehose.publish(Frame {
                seq,
                bytes: Bytes::from(frame(seq, false)),
            });
        }

        // Only 4, 5, 6 remain buffered.
        let (frames, outdated) = firehose.replay(Some(1)).await;
        assert!(outdated, "cursor 1 predates the buffer");
        assert_eq!(
            frames.iter().map(|f| f.seq).collect::<Vec<_>>(),
            vec![4, 5, 6]
        );

        // A cursor exactly at the edge is still honourable.
        let (_, outdated) = firehose.replay(Some(3)).await;
        assert!(!outdated);
    }

    #[tokio::test]
    async fn the_buffer_is_bounded() {
        let firehose = firehose(3).await;
        for seq in 1..=10 {
            firehose.publish(Frame {
                seq,
                bytes: Bytes::from(frame(seq, false)),
            });
        }
        assert_eq!(firehose.buffer.read().len(), 3);
    }

    #[tokio::test]
    async fn replay_survives_a_restart() {
        let store = Arc::new(Store::open_in_memory().await.unwrap());

        // A gateway that saw frames 1..=5 and persisted them.
        let frames: Vec<(i64, Vec<u8>)> = (1..=5).map(|seq| (seq, frame(seq, false))).collect();
        store.append_frames(&frames).await.unwrap();
        store.put_firehose_cursor("radxa", 5, 6).await.unwrap();

        // Restart: a fresh Firehose over the same store.
        let restarted = Firehose::new(config(8), store).await.unwrap();

        // The sequence continues rather than restarting at 1.
        assert_eq!(restarted.current_seq(), 5);

        // And a subscriber's existing cursor is still honoured, from disk.
        let (replayed, outdated) = restarted.replay(Some(2)).await;
        assert!(!outdated, "frames 3..=5 are still on disk");
        assert_eq!(
            replayed.iter().map(|f| f.seq).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        assert_eq!(read_seq(&replayed[0].bytes), Some(3));
    }

    #[tokio::test]
    async fn a_cursor_older_than_the_durable_log_is_outdated() {
        let store = Arc::new(Store::open_in_memory().await.unwrap());
        let frames: Vec<(i64, Vec<u8>)> = (10..=12).map(|seq| (seq, frame(seq, false))).collect();
        store.append_frames(&frames).await.unwrap();
        store.put_firehose_cursor("radxa", 12, 13).await.unwrap();

        let restarted = Firehose::new(config(8), store).await.unwrap();
        let (replayed, outdated) = restarted.replay(Some(1)).await;

        assert!(outdated, "nothing before 10 survives, so say so");
        assert_eq!(replayed.first().map(|f| f.seq), Some(10));
    }

    #[tokio::test]
    async fn live_subscribers_receive_published_frames() {
        let firehose = firehose(8).await;
        let mut rx = firehose.subscribe();

        firehose.publish(Frame {
            seq: 1,
            bytes: Bytes::from(frame(1, false)),
        });

        let received = rx.recv().await.unwrap();
        assert_eq!(received.seq, 1);
    }
}
