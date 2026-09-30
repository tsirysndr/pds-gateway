//! Operator API at `/admin`, gated by a bearer token.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{GatewayError, Result};
use crate::identity::Handle;
use crate::state::AppState;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/nodes", get(nodes))
        .route("/accounts", get(accounts))
        .route("/accounts/{did}", delete(forget_account))
        .route("/reservations", get(reservations))
        .route("/reservations/{handle}", delete(release_reservation))
        .route("/resolve", get(resolve))
        .route("/cache/purge", post(purge))
        .route("/nodes/{from}/drain/{to}", post(drain))
}

fn authorize(state: &Arc<AppState>, headers: &HeaderMap) -> Result<()> {
    let Some(expected) = state
        .config
        .admin
        .token
        .as_deref()
        .filter(|t| !t.is_empty())
    else {
        return Err(GatewayError::Forbidden(
            "the admin API is disabled; set admin.token to enable it".to_owned(),
        ));
    };

    if tokens_match(bearer(headers).unwrap_or_default(), expected) {
        Ok(())
    } else {
        Err(GatewayError::AuthRequired)
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Constant-time comparison, so the token cannot be guessed byte by byte.
fn tokens_match(presented: &str, expected: &str) -> bool {
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn nodes(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Response> {
    authorize(&state, &headers)?;

    let counts: std::collections::HashMap<String, u64> =
        state.store.counts_by_node().await?.into_iter().collect();
    let health: std::collections::HashMap<String, crate::health::NodeHealth> =
        state.fleet.snapshot().into_iter().collect();

    let nodes: Vec<Value> = state
        .config
        .nodes
        .iter()
        .map(|node| {
            let h = health.get(&node.name);
            json!({
                "name": node.name,
                "url": node.url.as_str(),
                "publicHost": node.effective_public_host(),
                "did": node.did,
                "weight": node.weight,
                "acceptsSignups": node.accepts_signups,
                "maxAccounts": node.max_accounts,
                "accounts": counts.get(&node.name).copied().unwrap_or(0),
                "up": h.map(|h| h.up),
                "latencyMs": h.and_then(|h| h.last_latency).map(|d| d.as_millis()),
                "lastError": h.and_then(|h| h.last_error.clone()),
            })
        })
        .collect();

    Ok(Json(json!({
        "placement": state.config.gateway.placement,
        "defaultNode": state.config.gateway.default_node,
        "nodes": nodes,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct AccountsQuery {
    node: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AccountsQuery>,
) -> Result<Response> {
    authorize(&state, &headers)?;

    let limit = query.limit.unwrap_or(100);
    let rows = state
        .store
        .list_accounts(query.node.as_deref(), query.cursor.as_deref(), limit)
        .await?;

    let cursor = rows.last().map(|a| a.did.clone());
    let accounts: Vec<Value> = rows
        .iter()
        .map(|a| {
            json!({
                "did": a.did,
                "handle": a.handle,
                "node": a.node,
                "status": a.status,
                "createdAt": a.created_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "total": state.store.total_accounts().await?,
        "cursor": cursor,
        "accounts": accounts,
    }))
    .into_response())
}

/// Removes the gateway's record of an account. The account itself is untouched;
/// the next request for it resolves over the network again.
async fn forget_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(did): Path<String>,
) -> Result<Response> {
    authorize(&state, &headers)?;

    let removed = state.store.delete_account(&did).await?;
    if let Ok(parsed) = crate::identity::Did::parse(&did) {
        state.resolver.invalidate_did(&parsed).await;
    }

    Ok(Json(json!({"forgotten": removed})).into_response())
}

async fn reservations(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Result<Response> {
    authorize(&state, &headers)?;

    let rows = state.store.list_reservations().await?;
    let reservations: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "handle": r.handle,
                "node": r.node,
                "owner": r.owner,
                "expiresAt": r.expires_at,
            })
        })
        .collect();

    Ok(Json(json!({"reservations": reservations})).into_response())
}

async fn release_reservation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(handle): Path<String>,
) -> Result<Response> {
    authorize(&state, &headers)?;

    let parsed = Handle::parse(&handle)?;
    let held = state
        .store
        .list_reservations()
        .await?
        .into_iter()
        .find(|r| r.handle.eq_ignore_ascii_case(parsed.as_str()));

    match held {
        Some(reservation) => {
            state
                .store
                .release_handle(&reservation.handle, &reservation.owner)
                .await?;
            state.coord.unlock_handle(&reservation.handle).await;
            Ok(Json(json!({"released": true})).into_response())
        }
        None => Ok((StatusCode::NOT_FOUND, Json(json!({"released": false}))).into_response()),
    }
}

#[derive(Deserialize)]
pub struct ResolveQuery {
    subject: String,
    /// Skip every cache and ask the nodes directly.
    #[serde(default)]
    live: bool,
}

/// Explains where a subject routes and why, which is the thing an operator
/// actually needs when a request lands on the wrong node.
async fn resolve(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ResolveQuery>,
) -> Result<Response> {
    authorize(&state, &headers)?;

    let Some(subject) = crate::routing::Subject::parse(&query.subject) else {
        return Err(GatewayError::InvalidRequest(
            "subject could not be parsed".to_owned(),
        ));
    };

    let mut answer = json!({
        "subject": query.subject,
        "kind": match &subject {
            crate::routing::Subject::Did(_) => "did",
            crate::routing::Subject::Handle(_) => "handle",
            crate::routing::Subject::Opaque(_) => "opaque",
        },
    });

    if let crate::routing::Subject::Handle(handle) = &subject {
        answer["ownedNamespace"] = json!(state.config.owns_handle(handle.as_str()));
        if state.config.owns_handle(handle.as_str())
            && let Some(claim) = state
                .router
                .resolve_owned_handle(
                    handle,
                    if query.live {
                        crate::routing::Lookup::Live
                    } else {
                        crate::routing::Lookup::Cached
                    },
                )
                .await?
        {
            answer["did"] = json!(claim.did.as_str());
            answer["claimedBy"] = json!(claim.node);
        }
    }

    answer["node"] = json!(state.router.locate(&subject).await?);
    Ok(Json(answer).into_response())
}

#[derive(Deserialize)]
pub struct PurgeBody {
    handle: Option<String>,
    did: Option<String>,
}

async fn purge(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PurgeBody>,
) -> Result<Response> {
    authorize(&state, &headers)?;

    let mut purged = Vec::new();

    if let Some(raw) = &body.handle {
        let handle = Handle::parse(raw)?;
        state.resolver.invalidate_handle(&handle).await;
        state.delegates.invalidate(&handle).await;
        purged.push(handle.into_string());
    }
    if let Some(raw) = &body.did {
        let did = crate::identity::Did::parse(raw)?;
        state.resolver.invalidate_did(&did).await;
        purged.push(did.into_string());
    }

    Ok(Json(json!({"purged": purged})).into_response())
}

/// Reassigns every account from one node to another, for retiring a node after
/// its repositories have been migrated.
async fn drain(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((from, to)): Path<(String, String)>,
) -> Result<Response> {
    authorize(&state, &headers)?;

    if state.config.node(&to).is_none() {
        return Err(GatewayError::InvalidRequest(format!(
            "unknown target node `{to}`"
        )));
    }

    let moved = state.store.move_node(&from, &to).await?;
    let hints = state.store.forget_hints_for_node(&from).await?;
    tracing::warn!(%from, %to, moved, "drained node");

    Ok(Json(json!({"moved": moved, "hintsCleared": hints})).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(token: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        if let Some(token) = token {
            map.insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn matches_only_the_exact_token() {
        assert!(tokens_match("hunter2", "hunter2"));
        assert!(!tokens_match("hunter2", "hunter3"));
        // A prefix must not pass.
        assert!(!tokens_match("hunter", "hunter2"));
        assert!(!tokens_match("hunter2x", "hunter2"));
        assert!(!tokens_match("", "hunter2"));
        // An empty expected token would otherwise accept an absent header.
        assert!(tokens_match("", ""));
    }

    #[test]
    fn reads_only_a_bearer_scheme() {
        assert_eq!(bearer(&headers(Some("hunter2"))), Some("hunter2"));
        assert_eq!(bearer(&headers(None)), None);

        let mut basic = HeaderMap::new();
        basic.insert(header::AUTHORIZATION, "Basic abc123".parse().unwrap());
        assert_eq!(bearer(&basic), None);
    }
}
