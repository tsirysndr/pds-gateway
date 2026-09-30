pub mod admin;
pub mod describe;
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

pub fn router(state: Arc<AppState>) -> AxumRouter {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(Duration::from_secs(600));

    AxumRouter::new()
        .route("/", get(describe::root))
        .route("/health", get(describe::health))
        .route("/health/ready", get(describe::ready))
        .route("/metrics", get(describe::metrics))
        // Delegate and identity endpoints. The nodes and the TLS issuer call
        // these, so they must answer without a redirect.
        .route("/.well-known/atproto-did", get(wellknown::atproto_did))
        .route("/.well-known/did.json", get(wellknown::did_json))
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
        .nest("/admin", admin::router())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
