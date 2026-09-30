//! `com.atproto.sync.subscribeRepos` for the whole fleet.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::SinkExt;
use serde::Deserialize;

use crate::config::FirehoseMode;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SubscribeQuery {
    pub cursor: Option<i64>,
    /// Pin the subscription to one node instead of the merged stream.
    pub node: Option<String>,
}

/// Registered in place of the real handler when the firehose is off, so the
/// refusal is reported before the WebSocket upgrade is attempted.
pub async fn disabled() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        "this gateway does not serve the firehose",
    )
        .into_response()
}

pub async fn subscribe_repos(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SubscribeQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    match state.config.firehose.mode {
        FirehoseMode::Off => disabled().await,

        FirehoseMode::Passthrough => {
            let name = query
                .node
                .clone()
                .unwrap_or_else(|| state.router.default_node());
            let Some(node) = state.config.node(&name).cloned() else {
                return (StatusCode::BAD_REQUEST, format!("unknown node `{name}`")).into_response();
            };
            let cursor = query
                .cursor
                .map(|c| format!("cursor={c}"))
                .unwrap_or_default();

            upgrade.on_upgrade(move |socket| {
                crate::proxy::ws::relay(
                    socket,
                    node,
                    "com.atproto.sync.subscribeRepos".to_owned(),
                    cursor,
                )
            })
        }

        FirehoseMode::Multiplex => {
            // A pinned node still gets a straight relay, which is what an
            // operator debugging one node wants.
            if let Some(name) = &query.node {
                let Some(node) = state.config.node(name).cloned() else {
                    return (StatusCode::BAD_REQUEST, format!("unknown node `{name}`"))
                        .into_response();
                };
                let cursor = query
                    .cursor
                    .map(|c| format!("cursor={c}"))
                    .unwrap_or_default();
                return upgrade.on_upgrade(move |socket| {
                    crate::proxy::ws::relay(
                        socket,
                        node,
                        "com.atproto.sync.subscribeRepos".to_owned(),
                        cursor,
                    )
                });
            }

            let Some(firehose) = state.firehose.clone() else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "the firehose is not running",
                )
                    .into_response();
            };
            upgrade.on_upgrade(move |socket| serve_merged(socket, firehose, query.cursor))
        }
    }
}

async fn serve_merged(
    mut socket: WebSocket,
    firehose: Arc<crate::firehose::Firehose>,
    cursor: Option<i64>,
) {
    // Subscribe before replaying, so frames produced during the replay are not
    // lost between the two.
    let mut live = firehose.subscribe();
    let (replay, outdated) = firehose.replay(cursor).await;

    if outdated
        && socket
            .send(Message::Binary(
                crate::firehose::outdated_cursor_frame().to_vec().into(),
            ))
            .await
            .is_err()
    {
        return;
    }

    let mut last_sent = cursor.unwrap_or(0);
    for frame in replay {
        if socket
            .send(Message::Binary(frame.bytes.to_vec().into()))
            .await
            .is_err()
        {
            return;
        }
        last_sent = frame.seq;
    }

    loop {
        match live.recv().await {
            Ok(frame) => {
                // Skip anything the replay already delivered.
                if frame.seq <= last_sent {
                    continue;
                }
                if socket
                    .send(Message::Binary(frame.bytes.to_vec().into()))
                    .await
                    .is_err()
                {
                    return;
                }
                last_sent = frame.seq;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                crate::metrics::FIREHOSE_LAGGED.incr();
                tracing::warn!(missed, "firehose subscriber fell behind; disconnecting");
                let _ = socket.close().await;
                return;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                let _ = socket.close().await;
                return;
            }
        }
    }
}
