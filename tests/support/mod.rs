//! Stub PDS nodes and a gateway wired to them.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
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
    /// Handles this node answers for but does not host, as a PDS acting as a
    /// delegate does when it relays its own delegates' answers.
    pub relays: Vec<Hosted>,
    /// (path, host, x-forwarded-proto) for every request the node received.
    pub seen: Vec<(String, Option<String>, Option<String>)>,
}

/// DID -> hosting public host, shared by the stub nodes and the stub PLC
/// directory. Keyed by DID, which the stubs derive from the handle, so a handle
/// always maps to the same document.
static PLC: std::sync::LazyLock<Arc<Mutex<HashMap<String, String>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

fn plc_register(did: &str, handle: &str, host: &str) {
    PLC.lock()
        .insert(did.to_owned(), format!("{host}|{handle}"));
}

/// A stand-in for plc.directory, so DID documents resolve without the network.
async fn start_plc() -> SocketAddr {
    let app = axum::Router::new().route(
        "/{did}",
        get(|axum::extract::Path(did): axum::extract::Path<String>| async move {
            match PLC.lock().get(&did) {
                Some(entry) => {
                    let (host, handle) = entry.split_once('|').unwrap_or((entry.as_str(), ""));
                    Json(json!({
                        "id": did,
                        "alsoKnownAs": [format!("at://{handle}")],
                        "service": [{
                            "id": "#atproto_pds",
                            "type": "AtprotoPersonalDataServer",
                            "serviceEndpoint": format!("https://{host}"),
                        }],
                    }))
                    .into_response()
                }
                None => (StatusCode::NOT_FOUND, "not found").into_response(),
            }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
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

    pub fn seen(&self) -> Vec<(String, Option<String>, Option<String>)> {
        self.inner.lock().seen.clone()
    }

    pub fn set_offline(&self, offline: bool) {
        self.inner.lock().offline = offline;
    }

    /// Makes this node answer resolveHandle for a handle it does not host. The
    /// DID document still names the real host.
    pub fn relay(&self, did: &str, handle: &str) {
        self.inner.lock().relays.push(Hosted {
            did: did.to_owned(),
            handle: handle.to_owned(),
            email: String::new(),
            password: String::new(),
        });
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
    let host = format!("{name}.rocksky.social");
    for account in &seed {
        plc_register(&account.did, &account.handle, &host);
    }
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
        .route(
            "/xrpc/com.atproto.server.describeServer",
            get(describe_server),
        )
        .route("/xrpc/com.atproto.server.getSession", get(get_session))
        // Routes a real PDS serves outside /xrpc, which the gateway must pass
        // through rather than shadow.
        .route("/", get(pds_home))
        .route("/health", get(pds_health))
        .route("/metrics", get(pds_metrics))
        .route("/oauth/par", post(pds_par))
        .route("/.well-known/did.json", get(pds_did_json))
        // Neighbours of the paths the console answers. The tests assert these
        // still reach the PDS.
        .route("/account/login", get(pds_owned))
        .route("/account/signup", get(pds_owned))
        .route("/account/sessions", get(pds_owned))
        .route("/account/security", get(pds_owned))
        .route("/oauth/authorize", get(pds_owned))
        .route("/assets/account.js", get(pds_owned))
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

fn record(state: &StubState, headers: &HeaderMap, path: &str) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    state.inner.lock().seen.push((
        path.to_owned(),
        header("host"),
        header("x-forwarded-proto"),
    ));
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

    let inner = state.inner.lock();
    match inner
        .accounts
        .iter()
        .chain(inner.relays.iter())
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

async fn create_account(
    State(state): State<StubState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    record(&state, &headers, "/xrpc/com.atproto.server.createAccount");
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
    plc_register(&did, &handle, &format!("{}.rocksky.social", state.name));

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

async fn create_session(
    State(state): State<StubState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    record(&state, &headers, "/xrpc/com.atproto.server.createSession");
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

async fn get_record(State(state): State<StubState>, headers: HeaderMap) -> Response {
    record(&state, &headers, "/xrpc/com.atproto.repo.getRecord");
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

async fn describe_server(State(state): State<StubState>, headers: HeaderMap) -> Response {
    record(&state, &headers, "/xrpc/com.atproto.server.describeServer");
    if let Some(down) = offline(&state) {
        return down;
    }
    Json(json!({
        "did": format!("did:web:{}.rocksky.social", state.name),
        "availableUserDomains": [".example.invalid"],
        "inviteCodeRequired": false,
        "blobUploadLimit": 5242880,
        "contact": {"email": "admin@rocksky.social"},
    }))
    .into_response()
}

/// Anything the PDS owns: a test asserts the gateway did not take it over.
async fn pds_owned(State(state): State<StubState>, headers: HeaderMap) -> Response {
    record(&state, &headers, "pds-owned");
    Json(json!({"servedBy": state.name, "owner": "pds"})).into_response()
}

async fn pds_home(State(state): State<StubState>, headers: HeaderMap) -> Response {
    record(&state, &headers, "/");
    // Plain text, like a PDS home page: ASCII art, not a web page.
    (
        [("content-type", "text/plain; charset=utf-8")],
        format!("   __\n  /  \\  {} pds\n", state.name),
    )
        .into_response()
}

async fn pds_health(State(state): State<StubState>) -> Response {
    Json(json!({"pds": state.name, "status": "ok"})).into_response()
}

async fn pds_metrics(State(state): State<StubState>) -> Response {
    (
        [("content-type", "text/plain")],
        format!("pds_up{{node=\"{}\"}} 1\n", state.name),
    )
        .into_response()
}

async fn pds_par(State(state): State<StubState>) -> Response {
    (
        StatusCode::CREATED,
        Json(json!({"request_uri": format!("urn:{}:abc", state.name)})),
    )
        .into_response()
}

async fn pds_did_json(State(state): State<StubState>) -> Response {
    Json(json!({"id": format!("did:web:{}.rocksky.social", state.name)})).into_response()
}

#[derive(serde::Deserialize)]
struct PageQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

async fn list_repos(
    State(state): State<StubState>,
    headers: HeaderMap,
    Query(page): Query<PageQuery>,
) -> Response {
    record(&state, &headers, "/xrpc/com.atproto.sync.listRepos");
    if let Some(down) = offline(&state) {
        return down;
    }

    let accounts = state.inner.lock().accounts.clone();
    // Cursor is an offset into this node's own list, in its own cursor space.
    let offset: usize = page
        .cursor
        .as_deref()
        .and_then(|c| c.strip_prefix(&format!("{}-", state.name)))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let limit = page.limit.unwrap_or(50).max(1);

    let repos: Vec<Value> = accounts
        .iter()
        .skip(offset)
        .take(limit)
        .map(|a| json!({"did": a.did, "head": "bafy", "node": state.name}))
        .collect();

    let consumed = offset + repos.len();
    let mut body = json!({"repos": repos});
    // Only hand back a cursor while there is more to read.
    if consumed < accounts.len() {
        body["cursor"] = json!(format!("{}-{}", state.name, consumed));
    }
    Json(body).into_response()
}

pub struct Harness {
    pub app: axum::Router,
    pub state: Arc<pds_gateway::AppState>,
    pub nodes: HashMap<String, StubNode>,
    _dir: tempfile::TempDir,
}

pub async fn harness(nodes: Vec<StubNode>, tweak: impl FnOnce(&mut Config)) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let plc = start_plc().await;

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
        server: pds_gateway::config::ServerConfig {
            // Production is HTTPS; the forwarded origin headers derive from this.
            public_url: url::Url::parse("https://rocksky.social").unwrap(),
            did: Some("did:web:rocksky.social".into()),
            ..Default::default()
        },
        gateway: GatewayConfig {
            handle_domains: vec!["rocksky.social".into()],
            default_node: Some(nodes[0].name.clone()),
            ..GatewayConfig::default()
        },
        identity: IdentityConfig {
            dns_resolution: false,
            well_known_resolution: false,
            plc_directory_url: url::Url::parse(&format!("http://{plc}")).unwrap(),
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
