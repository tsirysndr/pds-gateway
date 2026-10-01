//! The XRPC entry point.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Json, Response};
use bytes::Bytes;
use serde_json::{Value, json};

use crate::error::{GatewayError, Result};
use crate::identity::{Did, Handle};
use crate::proxy::forward::RequestBody;
use crate::routing::auth::TokenClaims;
use crate::routing::lexicon;
use crate::routing::router::{Decision, Lookup};
use crate::state::AppState;

pub async fn handle(
    State(state): State<Arc<AppState>>,
    Path(nsid): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response> {
    crate::metrics::REQUESTS.incr();

    let query_string = uri.query().unwrap_or("").to_owned();
    let query = parse_query(&query_string);
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(TokenClaims::from_authorization);

    let handling = lexicon::classify(&nsid);

    // Only buffer when the routing subject lives in the body, and never for
    // methods that stream gigabytes.
    let (buffered, body) = if handling.needs_body() && !lexicon::is_streaming(&nsid) {
        let bytes = read_limited(body, state.config.upstream.max_buffered_body_bytes).await?;
        let parsed = serde_json::from_slice::<Value>(&bytes).ok();
        (parsed, RequestBody::Buffered(bytes))
    } else {
        (None, RequestBody::Stream(body))
    };

    let decision = state
        .router
        .route(&nsid, &query, buffered.as_ref(), auth.as_ref(), &headers)
        .await?;

    tracing::debug!(nsid = %nsid, ?decision, "routed");

    match decision {
        Decision::Local => local(&state, &nsid, &query, &headers, body).await,
        Decision::Fanout => fanout(&state, &nsid, &method, &query_string, &headers, body).await,
        Decision::Broadcast { preferred } => {
            broadcast(
                &state,
                &nsid,
                &method,
                &query_string,
                &headers,
                body,
                preferred,
                buffered.as_ref(),
            )
            .await
        }
        Decision::Node { name, why } => {
            if !state.fleet.is_routable(&name) {
                return Err(GatewayError::NoNodeAvailable(format!(
                    "node `{name}` is down"
                )));
            }
            tracing::debug!(nsid = %nsid, node = %name, ?why, "forwarding");
            state
                .forwarder
                .forward(
                    &name,
                    method,
                    &nsid,
                    &query_string,
                    &headers,
                    body,
                    client_ip(&state, &headers),
                )
                .await
        }
    }
}

async fn local(
    state: &Arc<AppState>,
    nsid: &str,
    query: &[(String, String)],
    headers: &HeaderMap,
    body: RequestBody,
) -> Result<Response> {
    match nsid {
        "_health" => Ok(Json(json!({
            "version": env!("CARGO_PKG_VERSION"),
        }))
        .into_response()),

        "com.atproto.server.describeServer" => {
            Ok(crate::api::describe::describe_server(state).await)
        }

        "com.atproto.identity.resolveHandle" => {
            let handle = query
                .iter()
                .find(|(k, _)| k == "handle")
                .map(|(_, v)| v.clone());
            crate::api::wellknown::resolve_handle(state, headers, handle.as_deref()).await
        }

        "com.atproto.server.createAccount" => create_account(state, headers, body).await,

        other => Err(GatewayError::NotImplemented(other.to_owned())),
    }
}

/// Places a new account, holding its handle so no other node can take it.
async fn create_account(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    body: RequestBody,
) -> Result<Response> {
    if !state.config.gateway.allow_signups {
        return Err(GatewayError::Forbidden(
            "this gateway is not accepting new accounts".to_owned(),
        ));
    }

    let bytes = match body {
        RequestBody::Buffered(bytes) => bytes,
        RequestBody::Stream(body) => {
            read_limited(body, state.config.upstream.max_buffered_body_bytes).await?
        }
        RequestBody::Empty => Bytes::new(),
    };

    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|e| GatewayError::InvalidRequest(format!("body is not valid JSON: {e}")))?;

    let requested = payload
        .get("handle")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GatewayError::InvalidRequest("handle is required".to_owned()))?;
    let handle = Handle::parse(requested)?;

    if state.config.owns_handle(handle.as_str()) {
        let label = handle
            .label_under_any(&state.config.gateway.handle_domains)
            .unwrap_or_default();
        if state
            .config
            .gateway
            .reserved_handles
            .contains(&label.to_ascii_lowercase())
        {
            crate::metrics::HANDLE_COLLISIONS.incr();
            return Err(GatewayError::HandleTaken(handle.to_string()));
        }
    }

    // Ask the nodes live: the gateway's registry may not know a handle another
    // node issued directly.
    if let Some(existing) = state
        .router
        .resolve_owned_handle(&handle, Lookup::Live)
        .await?
    {
        tracing::info!(handle = %handle, did = %existing.did, "handle already claimed");
        crate::metrics::HANDLE_COLLISIONS.incr();
        return Err(GatewayError::HandleTaken(handle.to_string()));
    }

    let node = state.router.place().await?;
    let owner = reservation_owner(&handle, &payload);
    let ttl = state.config.gateway.reservation_ttl.get();

    // Redis first, so concurrent replicas cannot both pass the check; SQLite is
    // the durable arbiter behind it.
    if !state
        .coord
        .try_lock_handle(handle.as_str(), &owner, ttl)
        .await
    {
        crate::metrics::HANDLE_COLLISIONS.incr();
        return Err(GatewayError::HandleTaken(handle.to_string()));
    }

    let reservation = match state
        .store
        .reserve_handle(handle.as_str(), &node, &owner, ttl)
        .await
    {
        Ok(reservation) => reservation,
        Err(e) => {
            state.coord.unlock_handle(handle.as_str()).await;
            if matches!(e, GatewayError::HandleTaken(_)) {
                crate::metrics::HANDLE_COLLISIONS.incr();
            }
            return Err(e);
        }
    };

    tracing::info!(handle = %handle, node = %reservation.node, "creating account");

    let response = state
        .forwarder
        .forward_buffered(
            &reservation.node,
            Method::POST,
            "com.atproto.server.createAccount",
            "",
            headers,
            Some(bytes),
        )
        .await;

    let response = match response {
        Ok(response) => response,
        Err(e) => {
            release(state, &handle, &owner).await;
            return Err(e);
        }
    };

    if !response.status.is_success() {
        release(state, &handle, &owner).await;
        tracing::warn!(
            handle = %handle,
            node = %reservation.node,
            status = %response.status,
            "upstream refused account creation"
        );
        return Ok(passthrough(response));
    }

    let created: Option<Value> = response.json();
    let did = created
        .as_ref()
        .and_then(|v| v.get("did"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    // The node may have normalised the handle, so record what it returned.
    let final_handle = created
        .as_ref()
        .and_then(|v| v.get("handle"))
        .and_then(|v| v.as_str())
        .and_then(|h| Handle::parse(h).ok())
        .unwrap_or_else(|| handle.clone());

    match did {
        Some(did) => {
            if let Err(e) = state
                .store
                .commit_reservation(final_handle.as_str(), &owner, &did, &reservation.node)
                .await
            {
                // The account exists upstream; losing the registry row only
                // costs a resolution next time, so this must not fail the call.
                tracing::error!(did = %did, error = %e, "could not record new account");
            }
            state.coord.unlock_handle(handle.as_str()).await;
            state.delegates.invalidate(&final_handle).await;
            state.resolver.invalidate_handle(&final_handle).await;

            crate::metrics::ACCOUNTS_CREATED.incr();
            tracing::info!(did = %did, handle = %final_handle, node = %reservation.node, "account created");
        }
        None => {
            release(state, &handle, &owner).await;
            tracing::error!(
                node = %reservation.node,
                "upstream returned success without a DID"
            );
        }
    }

    Ok(passthrough(response))
}

fn reservation_owner(handle: &Handle, payload: &Value) -> String {
    // Tie the reservation to the request's own identity so a retry reuses it
    // rather than colliding with itself.
    let email = payload.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let key = payload.get("did").and_then(|v| v.as_str()).unwrap_or(email);
    if key.is_empty() {
        format!("handle:{handle}")
    } else {
        format!("subject:{key}")
    }
}

async fn release(state: &Arc<AppState>, handle: &Handle, owner: &str) {
    if let Err(e) = state.store.release_handle(handle.as_str(), owner).await {
        tracing::warn!(handle = %handle, error = %e, "could not release reservation");
    }
    state.coord.unlock_handle(handle.as_str()).await;
}

/// Merges array results from every node into one page, paginating correctly:
/// each node gets its own slice of the cursor and its own share of the limit, so
/// no record is dropped or repeated across pages.
async fn fanout(
    state: &Arc<AppState>,
    nsid: &str,
    method: &Method,
    query: &str,
    headers: &HeaderMap,
    _body: RequestBody,
) -> Result<Response> {
    use crate::routing::MergedCursor;

    let params = parse_query(query);
    let incoming = params
        .iter()
        .find(|(k, _)| k == "cursor")
        .and_then(|(_, v)| MergedCursor::decode(v));

    // A cursor names the nodes that still have results; without one, ask
    // everybody.
    let nodes: Vec<String> = state
        .config
        .nodes
        .iter()
        .map(|n| n.name.clone())
        .filter(|n| state.fleet.is_routable(n))
        .filter(|n| incoming.as_ref().is_none_or(|c| c.get(n).is_some()))
        .collect();

    if nodes.is_empty() {
        // An exhausted cursor is a normal end of pagination, not an error.
        if incoming.is_some() {
            return Ok(Json(json!({})).into_response());
        }
        return Err(GatewayError::NoNodeAvailable("no node is up".to_owned()));
    }

    let limit = params
        .iter()
        .find(|(k, _)| k == "limit")
        .and_then(|(_, v)| v.parse::<i64>().ok());
    let per_node_limit = MergedCursor::split_limit(limit, nodes.len());

    // Each node gets the same query with its own cursor and share of the limit.
    let queries: Vec<String> = nodes
        .iter()
        .map(|node| {
            let mut pairs: Vec<(String, String)> = params
                .iter()
                .filter(|(k, _)| k != "cursor" && k != "limit")
                .cloned()
                .collect();
            if let Some(cursor) = incoming.as_ref().and_then(|c| c.get(node)) {
                pairs.push(("cursor".to_owned(), cursor.to_owned()));
            }
            if let Some(limit) = per_node_limit {
                pairs.push(("limit".to_owned(), limit.to_string()));
            }
            serde_urlencoded_pairs(&pairs)
        })
        .collect();

    let calls = nodes.iter().zip(&queries).map(|(node, query)| {
        state
            .forwarder
            .forward_buffered(node, method.clone(), nsid, query, headers, None)
    });
    let results = futures::future::join_all(calls).await;

    let mut merged = serde_json::Map::new();
    let mut arrays: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    let mut next = MergedCursor::default();
    let mut ok = 0;

    for (node, result) in nodes.iter().zip(results) {
        match result {
            Ok(response) if response.status.is_success() => {
                ok += 1;
                let Some(Value::Object(object)) = response.json::<Value>() else {
                    continue;
                };
                for (key, value) in object {
                    match (key.as_str(), value) {
                        // Only a node that returned a cursor has more to give.
                        ("cursor", Value::String(cursor)) if !cursor.is_empty() => {
                            next.set(node, cursor);
                        }
                        ("cursor", _) => {}
                        (_, Value::Array(items)) => arrays.entry(key).or_default().extend(items),
                        (_, other) => {
                            merged.entry(key).or_insert(other);
                        }
                    }
                }
            }
            Ok(response) => {
                tracing::debug!(node = %node, status = %response.status, "fanout member failed");
                // A node that errored is not exhausted; keep its cursor so the
                // next page retries it instead of silently skipping its records.
                if let Some(cursor) = incoming.as_ref().and_then(|c| c.get(node)) {
                    next.set(node, cursor);
                }
            }
            Err(e) => {
                tracing::debug!(node = %node, error = %e, "fanout member failed");
                if let Some(cursor) = incoming.as_ref().and_then(|c| c.get(node)) {
                    next.set(node, cursor);
                }
            }
        }
    }

    if ok == 0 {
        return Err(GatewayError::NoNodeAvailable(format!(
            "every node failed to answer {nsid}"
        )));
    }

    for (key, items) in arrays {
        merged.insert(key, Value::Array(items));
    }
    if let Some(cursor) = next.encode() {
        merged.insert("cursor".to_owned(), Value::String(cursor));
    }

    Ok(Json(Value::Object(merged)).into_response())
}

fn serde_urlencoded_pairs(pairs: &[(String, String)]) -> String {
    let mut out = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        out.append_pair(k, v);
    }
    out.finish()
}

/// Tries each node until one accepts. Used for login, where the identifier may
/// be an email the gateway cannot resolve to a node.
#[allow(clippy::too_many_arguments)]
async fn broadcast(
    state: &Arc<AppState>,
    nsid: &str,
    method: &Method,
    query: &str,
    headers: &HeaderMap,
    body: RequestBody,
    preferred: Option<String>,
    payload: Option<&Value>,
) -> Result<Response> {
    let bytes = match body {
        RequestBody::Buffered(bytes) => bytes,
        RequestBody::Stream(body) => {
            read_limited(body, state.config.upstream.max_buffered_body_bytes).await?
        }
        RequestBody::Empty => Bytes::new(),
    };

    let order = if state.config.gateway.broadcast_login {
        state.fleet.broadcast_order(preferred.as_deref())
    } else {
        vec![preferred.unwrap_or_else(|| state.router.default_node())]
    };

    let identifier = payload
        .and_then(|p| p.get("identifier"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let mut last: Option<crate::proxy::UpstreamResponse> = None;

    for node in &order {
        if !state.fleet.is_routable(node) {
            continue;
        }

        let response = match state
            .forwarder
            .forward_buffered(
                node,
                method.clone(),
                nsid,
                query,
                headers,
                Some(bytes.clone()),
            )
            .await
        {
            Ok(response) => response,
            Err(e) => {
                tracing::debug!(node = %node, error = %e, "broadcast member failed");
                continue;
            }
        };

        if response.status.is_success() {
            // Remember where this identifier lives, so the next login is a
            // single request rather than a sweep.
            if let Some(identifier) = &identifier {
                learn_login(state, identifier, node, &response).await;
            }
            return Ok(passthrough(response));
        }

        // A definite "wrong password" from the node that holds the account is
        // the real answer; keep looking only while nodes say "unknown account".
        if !is_unknown_account(&response) {
            return Ok(passthrough(response));
        }
        last = Some(response);
    }

    match last {
        Some(response) => Ok(passthrough(response)),
        None => Err(GatewayError::NoNodeAvailable(format!(
            "no node could answer {nsid}"
        ))),
    }
}

async fn learn_login(
    state: &Arc<AppState>,
    identifier: &str,
    node: &str,
    response: &crate::proxy::UpstreamResponse,
) {
    if let Err(e) = state.store.put_hint(identifier, node).await {
        tracing::warn!(error = %e, "could not record login hint");
    }

    // A successful login also tells us the account's DID and handle.
    if let Some(session) = response.json::<Value>() {
        let did = session.get("did").and_then(|v| v.as_str());
        let handle = session.get("handle").and_then(|v| v.as_str());
        if let (Some(did), Some(handle)) = (did, handle)
            && Did::parse(did).is_ok()
            && let Err(e) = state.store.upsert_account(did, handle, node).await
        {
            tracing::warn!(error = %e, "could not record account from login");
        }
    }
}

fn is_unknown_account(response: &crate::proxy::UpstreamResponse) -> bool {
    if response.status == StatusCode::UNAUTHORIZED {
        return true;
    }
    response
        .json::<Value>()
        .and_then(|v| {
            v.get("error").and_then(|e| e.as_str()).map(|e| {
                matches!(
                    e,
                    "AccountNotFound" | "InvalidRequest" | "AuthenticationRequired"
                )
            })
        })
        .unwrap_or(false)
}

fn passthrough(response: crate::proxy::UpstreamResponse) -> Response {
    let mut builder = Response::builder().status(response.status);
    for (name, value) in response.headers.iter() {
        if !matches!(
            name.as_str(),
            "connection" | "transfer-encoding" | "content-length" | "keep-alive"
        ) {
            builder = builder.header(name, value);
        }
    }
    builder = builder.header("x-pdsgw-node", &response.node);
    builder
        .body(Body::from(response.body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn read_limited(body: Body, limit: usize) -> Result<Bytes> {
    use http_body_util::BodyExt;
    let collected = body
        .collect()
        .await
        .map_err(|e| GatewayError::InvalidRequest(format!("could not read body: {e}")))?;
    let bytes = collected.to_bytes();
    if bytes.len() > limit {
        return Err(GatewayError::BodyTooLarge { limit });
    }
    Ok(bytes)
}

/// The caller's address, taken from `X-Forwarded-For` only when the reverse
/// proxy in front of the gateway is trusted.
pub fn client_ip(state: &Arc<AppState>, headers: &HeaderMap) -> Option<std::net::IpAddr> {
    if !state.config.server.trust_forwarded_headers {
        return None;
    }
    headers
        .get("x-forwarded-for")?
        .to_str()
        .ok()?
        .split(',')
        .next()?
        .trim()
        .parse()
        .ok()
}

pub fn parse_query(raw: &str) -> Vec<(String, String)> {
    if raw.is_empty() {
        return Vec::new();
    }
    url::form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repeated_and_encoded_query_parameters() {
        let query = parse_query("repo=did%3Aplc%3Aalice&collection=app.bsky.feed.post&limit=10");
        assert_eq!(query[0], ("repo".into(), "did:plc:alice".into()));
        assert_eq!(query[1].1, "app.bsky.feed.post");
        assert!(parse_query("").is_empty());
    }

    #[test]
    fn owner_is_stable_for_a_retry_and_distinct_per_subject() {
        let handle = Handle::parse("alice.rocksky.social").unwrap();
        let a = json!({"handle": "alice.rocksky.social", "email": "alice@example.com"});
        let b = json!({"handle": "alice.rocksky.social", "email": "bob@example.com"});

        assert_eq!(
            reservation_owner(&handle, &a),
            reservation_owner(&handle, &a)
        );
        assert_ne!(
            reservation_owner(&handle, &a),
            reservation_owner(&handle, &b)
        );

        // With nothing to key on, the handle itself is the owner.
        let bare = json!({"handle": "alice.rocksky.social"});
        assert_eq!(
            reservation_owner(&handle, &bare),
            "handle:alice.rocksky.social"
        );
    }
}
