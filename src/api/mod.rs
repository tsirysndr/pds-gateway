pub mod admin;
pub mod describe;
pub mod passthrough;
pub mod signin;
pub mod subscribe;
pub mod wellknown;
pub mod xrpc;

use std::sync::Arc;
use std::time::Duration;

use axum::Router as AxumRouter;
use axum::routing::{any, get};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// The gateway owns only what it must be authoritative for: XRPC, the handle
/// document and the TLS ask. Everything else on the hostname — OAuth, the
/// account frontend, `/metrics`, `did.json`, static assets — belongs to the PDS
/// and is passed through untouched, so putting the gateway in front of an
/// existing PDS does not take those routes away.
pub fn router(state: Arc<AppState>) -> AxumRouter {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(Duration::from_secs(600));

    let mut app = AxumRouter::new()
        .route("/.well-known/atproto-did", get(wellknown::atproto_did))
        .route("/tls-check", get(wellknown::tls_check))
        .route(
            "/xrpc/com.atproto.sync.subscribeRepos",
            if state.config.firehose.mode == crate::config::FirehoseMode::Off {
                get(subscribe::disabled)
            } else {
                get(subscribe::subscribe_repos)
            },
        )
        .route("/xrpc/{nsid}", any(xrpc::handle))
        // Gateway status lives under its own prefix so it cannot shadow a route
        // the PDS already serves.
        .route("/_gateway", get(describe::root))
        .route("/_gateway/health", get(describe::health))
        .route("/_gateway/health/ready", get(describe::ready))
        .route("/_gateway/metrics", get(describe::metrics))
        .nest("/_gateway/admin", admin::router())
        .fallback(any(passthrough::handle));

    // Sign-in is intercepted so the form can be sent to the PDS that holds the
    // account; everything else about these paths is the PDS's own.
    for path in &state.config.gateway.signin_paths {
        app = app.route(path, any(signin::handle));
    }

    app.layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
