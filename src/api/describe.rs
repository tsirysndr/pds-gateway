//! Gateway-level status and service description.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::state::AppState;

pub async fn root(State(state): State<Arc<AppState>>) -> Response {
    Json(json!({
        "service": "pds-gateway",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "A virtual AT Protocol PDS routing to the node that hosts each account",
        "publicUrl": state.config.server.public_url.as_str(),
        "handleDomains": state.config.gateway.handle_domains,
        "nodes": state.config.nodes.len(),
    }))
    .into_response()
}

pub async fn health(State(state): State<Arc<AppState>>) -> Response {
    let nodes: Vec<Value> = state
        .fleet
        .snapshot()
        .into_iter()
        .map(|(name, health)| {
            json!({
                "name": name,
                "up": health.up,
                "latencyMs": health.last_latency.map(|d| d.as_millis()),
                "failures": health.consecutive_failures,
                "lastError": health.last_error,
            })
        })
        .collect();

    let up = nodes.iter().filter(|n| n["up"] == json!(true)).count();

    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "nodesUp": up,
        "nodesTotal": nodes.len(),
        "redis": state.coord.is_shared(),
        "firehose": state.firehose.as_ref().map(|f| json!({"seq": f.current_seq()})),
        "nodes": nodes,
    }))
    .into_response()
}

/// Ready once at least one node can take traffic.
pub async fn ready(State(state): State<Arc<AppState>>) -> Response {
    let up = state
        .fleet
        .snapshot()
        .into_iter()
        .filter(|(_, health)| health.up)
        .count();

    if up == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ready": false, "reason": "no upstream node is up"})),
        )
            .into_response();
    }
    Json(json!({"ready": true, "nodesUp": up})).into_response()
}

pub async fn metrics(State(state): State<Arc<AppState>>) -> Response {
    if !state.config.admin.metrics {
        return (StatusCode::NOT_FOUND, "metrics are disabled").into_response();
    }

    let counts: std::collections::HashMap<String, u64> = state
        .store
        .counts_by_node()
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    let nodes: Vec<(String, bool, u64)> = state
        .fleet
        .snapshot()
        .into_iter()
        .map(|(name, health)| {
            let accounts = counts.get(&name).copied().unwrap_or(0);
            (name, health.up, accounts)
        })
        .collect();

    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        crate::metrics::render(&nodes),
    )
        .into_response()
}

/// `com.atproto.server.describeServer` for the fleet as one PDS.
pub fn describe_server(state: &Arc<AppState>) -> Json<Value> {
    Json(json!({
        "did": state.config.server.did,
        "availableUserDomains": state
            .config
            .gateway
            .handle_domains
            .iter()
            .map(|d| format!(".{d}"))
            .collect::<Vec<_>>(),
        "inviteCodeRequired": true,
        "links": {},
        "contact": {},
    }))
}
