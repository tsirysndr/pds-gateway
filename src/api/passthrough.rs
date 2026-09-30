//! Everything the gateway does not own goes to one node untouched.
//!
//! The gateway fronts a hostname that a PDS already serves, so OAuth, the
//! account frontend, metrics and `did.json` must keep working. Those live on the
//! default node; a request identifying an account routes to that account's node
//! instead, so a node-hosted user reaches their own PDS.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Uri};
use axum::response::Response;

use crate::error::Result;
use crate::proxy::forward::RequestBody;
use crate::routing::auth::TokenClaims;
use crate::state::AppState;

pub async fn handle(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response> {
    let path = uri.path();
    let query = uri.query().unwrap_or("");

    // A bearer token still says whose PDS this belongs to, which matters for a
    // node-hosted account reaching an account-management page.
    let node = match token_node(&state, &headers).await {
        Some(node) => node,
        None => state.router.default_node(),
    };

    tracing::debug!(%path, node = %node, "passing through");
    state
        .forwarder
        .forward_path(
            &node,
            method,
            path,
            query,
            &headers,
            RequestBody::Stream(body),
            crate::api::xrpc::client_ip(&state, &headers),
        )
        .await
}

async fn token_node(state: &Arc<AppState>, headers: &HeaderMap) -> Option<String> {
    let claims = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(TokenClaims::from_authorization)?;
    let subject = crate::routing::Subject::parse(claims.sub.as_deref()?)?;

    match state.router.locate(&subject).await {
        Ok(node) => node,
        Err(e) => {
            tracing::debug!(error = %e, "could not locate token subject for passthrough");
            None
        }
    }
}
