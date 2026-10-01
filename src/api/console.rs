//! The bundled account console, compiled into the binary.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
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
    let index = if mount == "/" { "/".to_owned() } else { mount.clone() };

    let mut router = Router::new()
        // Assets live under the gateway's own namespace, not under the mount:
        // the console also answers paths like /account/login, and a relative
        // URL there would resolve somewhere nothing is served.
        .route("/_gateway/console/{*path}", get(serve_asset))
        .route(&index, get(serve_index))
        .route("/_gateway/pds", get(servers));

    // The console is one page, so a path under the mount that is not a bundled
    // file still serves it and lets the router inside decide.
    if mount != "/" {
        router = router
            .route(&format!("{mount}/"), get(serve_index))
            .route(&format!("{mount}/{{*path}}"), get(serve_index));
    }

    // Paths the console answers instead of the PDS. Its sign-in resolves the
    // handle to the account's own node, which the PDS's own page cannot do.
    for path in &state.config.ui.screens {
        if path != &index {
            router = router.route(path, get(serve_screen));
        }
    }

    router
}

/// A screen path, answered by the console only for a client that asked for
/// HTML.
///
/// `curl https://pds.example` is not a browser: it gets whatever the PDS serves
/// there, which for a PDS home page is plain text. A bare `*/*` is not taken as
/// a request for the console, so only something that names `text/html` — every
/// browser does — is given the page.
async fn serve_screen(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if wants_html(&headers) && !oauth_ceremony(&uri) {
        return serve_index().await;
    }

    match crate::api::passthrough::handle(State(state), method, uri, headers, body).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

/// A sign-in that belongs to the PDS, not the console.
///
/// OAuth holds its state in the PDS's own browser session: the request the
/// client parked, the CSRF token, the handle the client hinted. The console
/// cannot see any of it, so a sign-in it performed would leave the ceremony
/// stranded with no way back to the client. The PDS marks the redirects of its
/// own ceremony with `flow=oauth`, and those pages stay its own.
fn oauth_ceremony(uri: &Uri) -> bool {
    uri.query()
        .is_some_and(|query| query.split('&').any(|pair| pair == "flow=oauth"))
}

fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| {
            accept
                .split(',')
                .any(|part| part.trim().starts_with("text/html"))
        })
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
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
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
