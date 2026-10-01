//! The bundled account console, compiled into the binary.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use axum::routing::get;
use rust_embed::Embed;
use serde_json::json;

use crate::state::AppState;

#[derive(Embed)]
#[folder = "ui/dist"]
struct Assets;

pub fn router(state: &Arc<AppState>) -> Router<Arc<AppState>> {
    let mount = state.config.ui.mount.clone();
    let index = if mount == "/" {
        "/".to_owned()
    } else {
        mount.clone()
    };

    let mut router = Router::new()
        .route(&index, get(serve_index))
        .route("/_gateway/pds", get(servers));

    // The console is a single page, so any path under the mount that is not a
    // bundled file still serves the page and lets the router inside it decide.
    if mount == "/" {
        router = router.route("/{*path}", get(serve_asset));
    } else {
        router = router
            .route(&format!("{mount}/"), get(serve_index))
            .route(&format!("{mount}/{{*path}}"), get(serve_asset));
    }

    router
}

async fn serve_index() -> Response {
    match Assets::get("index.html") {
        Some(file) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            file.data.into_owned(),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "the console was not bundled into this build",
        )
            .into_response(),
    }
}

async fn serve_asset(Path(path): Path<String>) -> Response {
    let trimmed = path.trim_start_matches('/');

    match Assets::get(trimmed) {
        Some(file) => {
            let mime = mime_guess::from_path(trimmed).first_or_octet_stream();
            // Hashed filenames are safe to cache hard; index.html is not.
            let cache = if trimmed.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, cache),
                ],
                file.data.into_owned(),
            )
                .into_response()
        }
        // Unknown path inside the console: serve the page, not a 404.
        None => serve_index().await,
    }
}

/// The fleet, for the console's server picker.
///
/// Only public URLs are listed. A node the gateway reaches privately is
/// published as the gateway's own origin, so choosing it sends requests here and
/// the gateway forwards them — an address only this process can reach is never
/// handed to a browser.
async fn servers(State(state): State<Arc<AppState>>) -> Response {
    let public_url = &state.config.server.public_url;
    let public = public_url.as_str().trim_end_matches('/');

    // Compare on host *and* port: a development gateway on localhost:4600 is not
    // the same authority as localhost:443.
    let own_authority = match (public_url.host_str(), public_url.port()) {
        (Some(host), Some(port)) => Some(format!("{host}:{port}")),
        (Some(host), None) => Some(host.to_owned()),
        (None, _) => None,
    };

    let servers: Vec<_> = state
        .config
        .nodes
        .iter()
        .map(|node| {
            let host = node.effective_public_host();
            let url = if own_authority.as_deref() == Some(host.as_str()) {
                // Reached through this gateway, so publish exactly how this
                // gateway is reached, scheme included.
                public.to_owned()
            } else {
                format!("https://{host}")
            };
            json!({
                "name": node.name,
                "url": url,
                "acceptsSignups": node.accepts_signups,
            })
        })
        .collect();

    Json(json!({
        "servers": servers,
        "handleDomains": state.config.gateway.handle_domains,
    }))
    .into_response()
}
