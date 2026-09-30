//! Stub PDS nodes and a gateway wired to them.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use parking_lot::Mutex;
use serde_json::{Value, json};

use pds_gateway::config::{
    AdminConfig, Config, DelegateConfig, FirehoseConfig, FirehoseMode, GatewayConfig, HealthConfig,
    IdentityConfig, NodeConfig, StoreConfig,
};

#[derive(Clone)]
pub struct Hosted {
    pub did: String,
    pub handle: String,
    pub email: String,
    pub password: String,
}

#[derive(Clone, Default)]
pub struct NodeInner {
    pub accounts: Vec<Hosted>,
    pub created: Vec<Value>,
    pub offline: bool,
}

#[derive(Clone)]
pub struct StubNode {
    pub name: String,
    pub did: String,
    pub addr: SocketAddr,
    pub inner: Arc<Mutex<NodeInner>>,
}

impl StubNode {
    #[allow(dead_code)]
    pub fn accounts(&self) -> Vec<Hosted> {
        self.inner.lock().accounts.clone()
    }

    pub fn created(&self) -> Vec<Value> {
        self.inner.lock().created.clone()
    }

    pub fn set_offline(&self, offline: bool) {
        self.inner.lock().offline = offline;
    }

    #[allow(dead_code)]
    pub fn add(&self, did: &str, handle: &str, email: &str, password: &str) {
        self.inner.lock().accounts.push(Hosted {
            did: did.to_owned(),
            handle: handle.to_owned(),
            email: email.to_owned(),
            password: password.to_owned(),
        });
    }
}

#[derive(Clone)]
struct StubState {
    name: String,
    inner: Arc<Mutex<NodeInner>>,
}

pub async fn start_node(name: &str, seed: Vec<Hosted>) -> StubNode {
    let inner = Arc::new(Mutex::new(NodeInner {
        accounts: seed,
        ..Default::default()
    }));
    let state = StubState {
        name: name.to_owned(),
        inner: inner.clone(),
    };

    let app = axum::Router::new()
        .route("/xrpc/_health", get(health))
        .route(
            "/xrpc/com.atproto.identity.resolveHandle",
            get(resolve_handle),
        )
        .route(
            "/xrpc/com.atproto.server.createAccount",
            post(create_account),
        )
        .route(
            "/xrpc/com.atproto.server.createSession",
            post(create_session),
        )
        .route("/xrpc/com.atproto.repo.getRecord", get(get_record))
        .route("/xrpc/com.atproto.sync.listRepos", get(list_repos))
        .route("/xrpc/com.atproto.server.getSession", get(get_session))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    StubNode {
        name: name.to_owned(),
        did: format!("did:web:{name}.rocksky.social"),
        addr,
        inner,
    }
}

fn offline(state: &StubState) -> Option<Response> {
    state
        .inner
        .lock()
        .offline
        .then(|| (StatusCode::SERVICE_UNAVAILABLE, "offline").into_response())
}

async fn health(State(state): State<StubState>) -> Response {
    if let Some(down) = offline(&state) {
        return down;
    }
    Json(json!({"version": "stub"})).into_response()
}

#[derive(serde::Deserialize)]
struct HandleQuery {
    handle: Option<String>,
}

async fn resolve_handle(
    State(state): State<StubState>,
    Query(query): Query<HandleQuery>,
) -> Response {
    if let Some(down) = offline(&state) {
        return down;
    }
    let wanted = query.handle.unwrap_or_default();

    match state
        .inner
        .lock()
        .accounts
        .iter()
        .find(|a| a.handle.eq_ignore_ascii_case(&wanted))
    {
        Some(account) => Json(json!({"did": account.did})).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "UnableToResolveHandle"})),
        )
            .into_response(),
    }
}

async fn create_account(State(state): State<StubState>, Json(body): Json<Value>) -> Response {
    if let Some(down) = offline(&state) {
        return down;
    }

    let handle = body
        .get("handle")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();

    let mut inner = state.inner.lock();
    if inner
        .accounts
        .iter()
        .any(|a| a.handle.eq_ignore_ascii_case(&handle))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "HandleNotAvailable"})),
        )
            .into_response();
    }

    let did = stub_did(&handle);
    inner.accounts.push(Hosted {
        did: did.clone(),
        handle: handle.clone(),
        email: body
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned(),
        password: body
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned(),
    });
    inner.created.push(body.clone());

    Json(json!({
        "did": did,
        "handle": handle,
        "accessJwt": "stub-access",
        "refreshJwt": "stub-refresh",
    }))
    .into_response()
}

/// A 24-character base32-sortable identifier derived from the handle, so the
/// delegate check accepts it and distinct handles never collide.
pub fn stub_did(handle: &str) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";

    let mut hash: u128 = 0xcbf2_9ce4_8422_2325;
    for byte in handle.as_bytes() {
        hash = hash.wrapping_mul(0x100_0000_01b3) ^ u128::from(*byte);
    }

    let id: String = (0..24)
        .map(|i| {
            let shift = (i * 5) % 96;
            char::from(ALPHABET[((hash >> shift) & 0x1f) as usize])
        })
        .collect();
    format!("did:plc:{id}")
}

async fn create_session(State(state): State<StubState>, Json(body): Json<Value>) -> Response {
    if let Some(down) = offline(&state) {
        return down;
    }

    let identifier = body
        .get("identifier")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    let inner = state.inner.lock();
    let found = inner.accounts.iter().find(|a| {
        a.handle.eq_ignore_ascii_case(identifier)
            || a.email.eq_ignore_ascii_case(identifier)
            || a.did == identifier
    });

    match found {
        Some(account) if account.password == password => Json(json!({
            "did": account.did,
            "handle": account.handle,
            "accessJwt": "stub-access",
            "refreshJwt": "stub-refresh",
            "servedBy": state.name,
        }))
        .into_response(),
        Some(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "InvalidPassword", "message": "wrong password"})),
        )
            .into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "AccountNotFound"})),
        )
            .into_response(),
    }
}

async fn get_record(State(state): State<StubState>) -> Response {
    if let Some(down) = offline(&state) {
        return down;
    }
    Json(json!({"servedBy": state.name})).into_response()
}

async fn get_session(State(state): State<StubState>) -> Response {
    if let Some(down) = offline(&state) {
        return down;
    }
    Json(json!({"servedBy": state.name})).into_response()
}

async fn list_repos(State(state): State<StubState>) -> Response {
    if let Some(down) = offline(&state) {
        return down;
    }
    let repos: Vec<Value> = state
        .inner
        .lock()
        .accounts
        .iter()
        .map(|a| json!({"did": a.did, "head": "bafy", "node": state.name}))
        .collect();

    Json(json!({"repos": repos, "cursor": format!("{}-cursor", state.name)})).into_response()
}

pub struct Harness {
    pub app: axum::Router,
    pub state: Arc<pds_gateway::AppState>,
    pub nodes: HashMap<String, StubNode>,
    _dir: tempfile::TempDir,
}

pub async fn harness(nodes: Vec<StubNode>, tweak: impl FnOnce(&mut Config)) -> Harness {
    let dir = tempfile::tempdir().unwrap();

    let node_configs: Vec<NodeConfig> = nodes
        .iter()
        .map(|n| NodeConfig {
            name: n.name.clone(),
            url: url::Url::parse(&format!("http://{}", n.addr)).unwrap(),
            public_host: Some(format!("{}.rocksky.social", n.name)),
            did: Some(n.did.clone()),
            weight: 1,
            accepts_signups: true,
            max_accounts: None,
        })
        .collect();

    let mut config = Config {
        gateway: GatewayConfig {
            handle_domains: vec!["rocksky.social".into()],
            default_node: Some(nodes[0].name.clone()),
            ..GatewayConfig::default()
        },
        identity: IdentityConfig {
            dns_resolution: false,
            well_known_resolution: false,
            ..IdentityConfig::default()
        },
        health: HealthConfig {
            timeout: std::time::Duration::from_millis(300).into(),
            failure_threshold: 1,
            success_threshold: 1,
            ..HealthConfig::default()
        },
        firehose: FirehoseConfig {
            mode: FirehoseMode::Off,
            ..FirehoseConfig::default()
        },
        delegate: DelegateConfig {
            ask_timeout: std::time::Duration::from_millis(500).into(),
            ..DelegateConfig::default()
        },
        store: StoreConfig {
            path: dir.path().join("gateway.sqlite3"),
            ..StoreConfig::default()
        },
        admin: AdminConfig {
            token: Some("test-admin-token".into()),
            metrics: true,
        },
        nodes: node_configs,
        ..Config::default()
    };
    tweak(&mut config);

    let state = pds_gateway::AppState::build(config).await.unwrap();
    let app = pds_gateway::api::router(state.clone());

    Harness {
        app,
        state,
        nodes: nodes.into_iter().map(|n| (n.name.clone(), n)).collect(),
        _dir: dir,
    }
}

impl Harness {
    pub async fn send(
        &self,
        request: axum::http::Request<axum::body::Body>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        use tower::ServiceExt;
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec();
        (status, headers, body)
    }

    pub async fn get(&self, uri: &str) -> (StatusCode, axum::http::HeaderMap, Value) {
        let (status, headers, body) = self.get_with(uri, &[]).await;
        (status, headers, parse(&body))
    }

    pub async fn get_with(
        &self,
        uri: &str,
        header_pairs: &[(&str, &str)],
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut builder = axum::http::Request::builder().uri(uri);
        for (k, v) in header_pairs {
            builder = builder.header(*k, *v);
        }
        self.send(builder.body(axum::body::Body::empty()).unwrap())
            .await
    }

    pub async fn post(&self, uri: &str, body: Value) -> (StatusCode, axum::http::HeaderMap, Value) {
        self.post_with(uri, body, &[]).await
    }

    pub async fn post_with(
        &self,
        uri: &str,
        body: Value,
        header_pairs: &[(&str, &str)],
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        for (k, v) in header_pairs {
            builder = builder.header(*k, *v);
        }
        let (status, headers, raw) = self
            .send(
                builder
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await;
        (status, headers, parse(&raw))
    }

    pub fn node_header(headers: &axum::http::HeaderMap) -> Option<String> {
        headers
            .get("x-pdsgw-node")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }
}

fn parse(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or_else(|_| json!({"_raw": String::from_utf8_lossy(body)}))
}

pub fn token(sub: &str, aud: Option<&str>) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let mut claims = json!({"sub": sub, "scope": "com.atproto.access"});
    if let Some(aud) = aud {
        claims["aud"] = json!(aud);
    }
    format!(
        "{}.{}.unchecked",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"at+jwt"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}
