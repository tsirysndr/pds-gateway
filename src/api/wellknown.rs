//! Endpoints the PDS nodes and the TLS issuer call.
//!
//! The gateway owns the wildcard for its handle domains, so it is the authority
//! for every handle in them. A node that does not host an account asks the
//! gateway, and the gateway asks the other nodes; whichever one holds the
//! account answers, and the name is therefore never issued twice.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::json;

use crate::error::Result;
use crate::identity::{Handle, delegate};
use crate::routing::Lookup;
use crate::state::AppState;

fn no_store() -> [(header::HeaderName, &'static str); 2] {
    [
        (header::CACHE_CONTROL, "no-store"),
        (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
    ]
}

/// `GET /.well-known/atproto-did` with the handle in the `Host` header.
/// Answers `200 text/plain <did>` or `404`, matching what a PDS serves.
pub async fn atproto_did(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response> {
    let host = request_host(&headers).unwrap_or_default();

    let Ok(handle) = Handle::parse(&host) else {
        return Ok((StatusCode::NOT_FOUND, no_store(), "Not found").into_response());
    };
    if !state.config.owns_handle(handle.as_str()) {
        return Ok((StatusCode::NOT_FOUND, no_store(), "Not found").into_response());
    }

    match state
        .router
        .resolve_owned_handle(&handle, Lookup::Cached)
        .await?
    {
        Some(claim) => Ok((
            StatusCode::OK,
            no_store(),
            [(header::CONTENT_TYPE, "text/plain")],
            claim.did.into_string(),
        )
            .into_response()),
        None => Ok((StatusCode::NOT_FOUND, no_store(), "Not found").into_response()),
    }
}

#[derive(Deserialize)]
pub struct ResolveHandleQuery {
    pub handle: Option<String>,
}

/// The delegate ask: `GET /xrpc/com.atproto.identity.resolveHandle?handle=`.
///
/// `live` is true on the allocation path, where a cached answer could refuse a
/// name that has since been given up.
pub async fn resolve_handle(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    handle: Option<&str>,
) -> Result<Response> {
    let Some(raw) = handle else {
        return Ok(invalid_handle("handle is required"));
    };

    let Ok(handle) = Handle::parse(raw) else {
        return Ok(invalid_handle("Invalid handle."));
    };

    if state.config.owns_handle(handle.as_str()) {
        let lookup = if delegate::is_hop(headers) {
            Lookup::RegistryOnly
        } else {
            Lookup::Live
        };
        if let Some(claim) = state.router.resolve_owned_handle(&handle, lookup).await?
            && delegate::is_delegate_safe(&claim.did)
        {
            return Ok(Json(json!({ "did": claim.did.as_str() })).into_response());
        }
        return Ok(unresolvable());
    }

    // Outside our namespace, resolve over the network as any PDS would.
    match state.resolver.resolve_handle(&handle).await? {
        Some(did) => Ok(Json(json!({ "did": did.as_str() })).into_response()),
        None => Ok(unresolvable()),
    }
}

fn invalid_handle(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "InvalidRequest", "message": message})),
    )
        .into_response()
}

fn unresolvable() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": "UnableToResolveHandle",
            "message": "Unable to resolve handle."
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct TlsCheckQuery {
    pub domain: Option<String>,
}

/// On-demand TLS authorisation. A certificate issuer asks whether it should get
/// a certificate for `domain`; the gateway approves its own hostname and any
/// handle in its namespace that actually exists.
pub async fn tls_check(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TlsCheckQuery>,
) -> Result<Response> {
    let deny = || (StatusCode::NOT_FOUND, no_store(), "").into_response();

    if !state.config.delegate.tls_check {
        return Ok(deny());
    }

    let Some(domain) = query.domain else {
        return Ok(deny());
    };
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty() || domain.len() > 253 {
        return Ok(deny());
    }

    if state.config.server.public_url.host_str() == Some(domain.as_str()) {
        return Ok((StatusCode::OK, no_store(), "").into_response());
    }

    let Ok(handle) = Handle::parse(&domain) else {
        return Ok(deny());
    };
    if !state.config.owns_handle(handle.as_str()) {
        return Ok(deny());
    }

    match state
        .router
        .resolve_owned_handle(&handle, Lookup::Cached)
        .await?
    {
        Some(_) => Ok((StatusCode::OK, no_store(), "").into_response()),
        None => Ok(deny()),
    }
}

/// The gateway's own DID document, when it has a `did:web` identity.
pub async fn did_json(State(state): State<Arc<AppState>>) -> Response {
    let Some(did) = &state.config.server.did else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };

    Json(json!({
        "@context": ["https://www.w3.org/ns/did/v1"],
        "id": did,
        "service": [{
            "id": "#atproto_pds",
            "type": "AtprotoPersonalDataServer",
            "serviceEndpoint": state.config.server.public_url.as_str().trim_end_matches('/'),
        }]
    }))
    .into_response()
}

fn request_host(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))?
        .to_str()
        .ok()?;
    // Drop any port and take the first value of a comma-joined list.
    let host = raw.split(',').next()?.trim();
    let host = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    Some(host.trim_matches('.').to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn reads_the_handle_out_of_the_host_header() {
        assert_eq!(
            request_host(&headers(&[("host", "alice.rocksky.social")])).as_deref(),
            Some("alice.rocksky.social")
        );
        // A port must not become part of the handle.
        assert_eq!(
            request_host(&headers(&[("host", "alice.rocksky.social:8443")])).as_deref(),
            Some("alice.rocksky.social")
        );
        assert_eq!(
            request_host(&headers(&[("host", "ALICE.Rocksky.Social")])).as_deref(),
            Some("alice.rocksky.social")
        );
    }

    #[test]
    fn prefers_the_forwarded_host_behind_a_proxy() {
        let map = headers(&[
            ("host", "gateway.internal"),
            ("x-forwarded-host", "alice.rocksky.social"),
        ]);
        assert_eq!(request_host(&map).as_deref(), Some("alice.rocksky.social"));
    }

    #[test]
    fn takes_the_first_of_a_joined_forwarded_host() {
        let map = headers(&[("x-forwarded-host", "alice.rocksky.social, proxy.internal")]);
        assert_eq!(request_host(&map).as_deref(), Some("alice.rocksky.social"));
    }

    #[test]
    fn has_no_host_when_none_is_sent() {
        assert!(request_host(&HeaderMap::new()).is_none());
    }
}
