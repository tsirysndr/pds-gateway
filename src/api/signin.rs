//! Browser sign-in, sent to the PDS that holds the account.
//!
//! The sign-in form names the account in `identifier`, but the form itself was
//! rendered by one node and its CSRF token is only valid there. Proxying the
//! submission to a different node would therefore be rejected, so when the
//! account lives elsewhere the browser is redirected to that node's own sign-in
//! page with the identifier carried across as a hint. The password is then
//! entered on the PDS that can actually verify it.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;

use crate::error::{GatewayError, Result};
use crate::proxy::forward::RequestBody;
use crate::routing::Subject;
use crate::state::AppState;

/// Form and query field names that carry the account being signed in.
const IDENTIFIER_FIELDS: [&str; 2] = ["identifier", "login_hint"];

pub async fn handle(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response> {
    let path = uri.path().to_owned();
    let query = uri.query().unwrap_or("").to_owned();

    // Only a form submission needs its body read; a GET carries any hint in the
    // query string.
    let (identifier, body) = if method == Method::POST {
        let bytes = read_form(body, state.config.upstream.max_buffered_body_bytes).await?;
        let found = field_from(&bytes, &query);
        (found, RequestBody::Buffered(bytes))
    } else {
        (field_from(b"", &query), RequestBody::Stream(body))
    };

    if state.config.gateway.signin_redirect
        && let Some(identifier) = identifier.as_deref()
        && let Some(target) = owning_node_host(&state, identifier).await?
    {
        tracing::info!(
            %path,
            identifier,
            node = %target.0,
            "redirecting sign-in to the account's own PDS"
        );
        return Ok(redirect(&target.1, &path, identifier));
    }

    let node = state.router.default_node();
    state
        .forwarder
        .forward_path(
            &node,
            method,
            &path,
            &query,
            &headers,
            body,
            crate::api::xrpc::client_ip(&state, &headers),
        )
        .await
}

/// The node hosting `identifier` and its public host, or `None` when the
/// request should be handled where it arrived.
async fn owning_node_host(
    state: &Arc<AppState>,
    identifier: &str,
) -> Result<Option<(String, String)>> {
    let Some(subject) = Subject::parse(identifier) else {
        return Ok(None);
    };
    let Some(node) = state.router.locate(&subject).await? else {
        return Ok(None);
    };

    // Already the node that would serve this request anyway.
    if node == state.router.default_node() {
        return Ok(None);
    }

    let Some(config) = state.config.node(&node) else {
        return Ok(None);
    };
    let host = config.effective_public_host();

    // A node that publishes our own hostname is reached through this gateway, so
    // redirecting there would loop straight back here.
    if Some(host.as_str()) == state.config.server.public_url.host_str() {
        return Ok(None);
    }

    Ok(Some((node, host)))
}

fn redirect(host: &str, path: &str, identifier: &str) -> Response {
    let hint = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("login_hint", identifier)
        .finish();
    let location = format!("https://{host}{path}?{hint}");

    // 303 so the browser re-issues this as a GET: the password is not replayed
    // to another host, it is entered on the PDS that can verify it.
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
    )
        .into_response()
}

/// Reads the first identifier field present in the form body, then the query.
fn field_from(body: &[u8], query: &str) -> Option<String> {
    for source in [body, query.as_bytes()] {
        for field in IDENTIFIER_FIELDS {
            if let Some(value) = url::form_urlencoded::parse(source)
                .find(|(k, _)| k == field)
                .map(|(_, v)| v.trim().to_owned())
                .filter(|v| !v.is_empty())
            {
                return Some(value);
            }
        }
    }
    None
}

async fn read_form(body: Body, limit: usize) -> Result<Bytes> {
    use http_body_util::BodyExt;
    let bytes = body
        .collect()
        .await
        .map_err(|e| GatewayError::InvalidRequest(format!("could not read form: {e}")))?
        .to_bytes();
    if bytes.len() > limit {
        return Err(GatewayError::BodyTooLarge { limit });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_identifier_from_a_form() {
        let body = b"_csrf_token=abc&identifier=alice.rocksky.social&password=hunter2hunter2";
        assert_eq!(
            field_from(body, "").as_deref(),
            Some("alice.rocksky.social")
        );
    }

    #[test]
    fn reads_a_login_hint_from_the_query() {
        assert_eq!(
            field_from(b"", "login_hint=bob.rocksky.social").as_deref(),
            Some("bob.rocksky.social")
        );
    }

    #[test]
    fn decodes_percent_encoding_and_trims() {
        let body = b"identifier=%20alice%40example.com%20&password=x";
        assert_eq!(field_from(body, "").as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn prefers_the_body_over_the_query() {
        let body = b"identifier=from-body.rocksky.social";
        assert_eq!(
            field_from(body, "login_hint=from-query.rocksky.social").as_deref(),
            Some("from-body.rocksky.social")
        );
    }

    #[test]
    fn finds_nothing_when_there_is_nothing_to_find() {
        assert!(field_from(b"password=x&_csrf_token=y", "").is_none());
        assert!(field_from(b"identifier=", "").is_none());
        assert!(field_from(b"identifier=%20%20", "").is_none());
        assert!(field_from(b"", "").is_none());
    }

    #[test]
    fn tolerates_a_body_that_is_not_a_form() {
        // Must not panic on arbitrary bytes.
        assert!(field_from(b"\xff\xfe binary \x00", "").is_none());
        assert!(field_from(b"{\"identifier\":\"alice\"}", "").is_none());
    }

    #[test]
    fn builds_a_see_other_to_the_owning_pds() {
        let response = redirect(
            "radxa.rocksky.social",
            "/account/login",
            "alice@example.com",
        );
        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let location = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(
            location,
            "https://radxa.rocksky.social/account/login?login_hint=alice%40example.com"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
    }
}
