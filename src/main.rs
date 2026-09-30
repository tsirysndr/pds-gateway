use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use pds_gateway::{AppState, Config, api};

#[derive(Parser)]
#[command(
    name = "pds-gateway",
    version,
    about = "A virtual AT Protocol PDS that routes XRPC traffic to the node hosting each account"
)]
struct Cli {
    /// Configuration file. Also read from GATEWAY_CONFIG.
    #[arg(short, long, env = "GATEWAY_CONFIG")]
    config: Option<PathBuf>,

    /// Emit logs as JSON instead of text.
    #[arg(long, env = "GATEWAY_LOG_JSON")]
    log_json: bool,

    /// Validate the configuration and exit.
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log_json);

    let config = Config::load(cli.config.as_deref()).context("invalid configuration")?;

    for node in &config.nodes {
        tracing::info!(
            node = %node.name,
            url = %node.url,
            public_host = %node.effective_public_host(),
            accepts_signups = node.accepts_signups,
            "upstream node"
        );
    }
    tracing::info!(
        domains = ?config.gateway.handle_domains,
        placement = ?config.gateway.placement,
        firehose = ?config.firehose.mode,
        delegate = config.delegate.enabled,
        "gateway configured"
    );

    if cli.check {
        tracing::info!("configuration is valid");
        return Ok(());
    }

    let bind = config.server.bind;
    let grace = config.server.shutdown_grace.get();

    let state = AppState::build(config).await?;
    let tasks = state.spawn_background();
    let app = api::router(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("could not bind {bind}"))?;
    tracing::info!(%bind, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown(grace))
        .await
        .context("server failed")?;

    for task in tasks {
        task.abort();
    }
    tracing::info!("stopped");
    Ok(())
}

fn init_tracing(json: bool) {
    let filter = tracing_subscriber::EnvFilter::try_from_env("GATEWAY_LOG")
        .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| "pds_gateway=info,tower_http=warn,axum=info".into());

    let registry = tracing_subscriber::registry().with(filter);
    if json {
        registry
            .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
            .init();
    } else {
        registry
            .with(tracing_subscriber::fmt::layer().with_target(true))
            .init();
    }
}

async fn shutdown(grace: std::time::Duration) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    tracing::info!(?grace, "shutting down; draining in-flight requests");
}
