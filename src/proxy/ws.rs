//! WebSocket relay for XRPC subscriptions.

use axum::extract::ws::{Message as AxumMessage, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::config::NodeConfig;

/// Builds the upstream `wss://`/`ws://` URL for a subscription.
pub fn upstream_url(node: &NodeConfig, nsid: &str, query: &str) -> String {
    let base = node.url.as_str().trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };

    if query.is_empty() {
        format!("{ws_base}/xrpc/{nsid}")
    } else {
        format!("{ws_base}/xrpc/{nsid}?{query}")
    }
}

/// Pumps frames between a client socket and one upstream node until either side
/// closes.
pub async fn relay(client: WebSocket, node: NodeConfig, nsid: String, query: String) {
    let url = upstream_url(&node, &nsid, &query);

    let (upstream, _) = match tokio_tungstenite::connect_async(&url).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(node = %node.name, %url, error = %e, "subscription upstream refused");
            let mut client = client;
            let _ = client.close().await;
            return;
        }
    };

    tracing::info!(node = %node.name, nsid = %nsid, "relaying subscription");
    let (mut client_tx, mut client_rx) = client.split();
    let (mut up_tx, mut up_rx) = upstream.split();

    let downstream = async {
        while let Some(frame) = up_rx.next().await {
            let message = match frame {
                Ok(WsMessage::Binary(bytes)) => AxumMessage::Binary(bytes.to_vec().into()),
                Ok(WsMessage::Text(text)) => AxumMessage::Text(text.as_str().into()),
                Ok(WsMessage::Ping(p)) => AxumMessage::Ping(p.to_vec().into()),
                Ok(WsMessage::Pong(p)) => AxumMessage::Pong(p.to_vec().into()),
                Ok(WsMessage::Close(_)) => break,
                Ok(WsMessage::Frame(_)) => continue,
                Err(e) => {
                    tracing::debug!(error = %e, "upstream subscription error");
                    break;
                }
            };
            crate::metrics::FIREHOSE_FRAMES.incr();
            if client_tx.send(message).await.is_err() {
                break;
            }
        }
    };

    let upstream_pump = async {
        while let Some(Ok(message)) = client_rx.next().await {
            let frame = match message {
                AxumMessage::Binary(b) => WsMessage::Binary(b.to_vec()),
                AxumMessage::Text(t) => WsMessage::Text(t.as_str().into()),
                AxumMessage::Ping(p) => WsMessage::Ping(p.to_vec()),
                AxumMessage::Pong(p) => WsMessage::Pong(p.to_vec()),
                AxumMessage::Close(_) => break,
            };
            if up_tx.send(frame).await.is_err() {
                break;
            }
        }
    };

    tokio::select! {
        _ = downstream => {}
        _ = upstream_pump => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn node(url: &str) -> NodeConfig {
        NodeConfig {
            name: "radxa".into(),
            url: Url::parse(url).unwrap(),
            public_host: None,
            did: None,
            weight: 1,
            accepts_signups: true,
            max_accounts: None,
            signin_path: None,
        }
    }

    #[test]
    fn maps_http_schemes_onto_websocket_schemes() {
        assert_eq!(
            upstream_url(
                &node("http://radxa.rocksky.social"),
                "com.atproto.sync.subscribeRepos",
                ""
            ),
            "ws://radxa.rocksky.social/xrpc/com.atproto.sync.subscribeRepos"
        );
        assert_eq!(
            upstream_url(
                &node("https://radxa.rocksky.social"),
                "com.atproto.sync.subscribeRepos",
                "cursor=42"
            ),
            "wss://radxa.rocksky.social/xrpc/com.atproto.sync.subscribeRepos?cursor=42"
        );
    }
}
