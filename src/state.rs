//! Shared state and startup wiring.

use std::sync::Arc;

use crate::config::Config;
use crate::firehose::Firehose;
use crate::health::Fleet;
use crate::identity::{Delegates, Resolver};
use crate::proxy::Forwarder;
use crate::registry::coord::Coordinator;
use crate::registry::store::Store;
use crate::routing::Router;

pub struct AppState {
    pub config: Arc<Config>,
    pub store: Arc<Store>,
    pub coord: Coordinator,
    pub resolver: Arc<Resolver>,
    pub delegates: Arc<Delegates>,
    pub fleet: Arc<Fleet>,
    pub router: Arc<Router>,
    pub forwarder: Arc<Forwarder>,
    pub firehose: Option<Arc<Firehose>>,
}

impl AppState {
    pub async fn build(config: Config) -> anyhow::Result<Arc<Self>> {
        let config = Arc::new(config);

        let http = reqwest::Client::builder()
            .connect_timeout(config.upstream.connect_timeout.get())
            .pool_idle_timeout(config.upstream.pool_idle_timeout.get())
            .pool_max_idle_per_host(config.upstream.pool_max_idle_per_host)
            .user_agent(concat!("pds-gateway/", env!("CARGO_PKG_VERSION")))
            // Redirects would let an upstream move a request somewhere the
            // gateway did not choose.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let store = Arc::new(Store::open(&config.store).await?);
        let coord = Coordinator::connect(&config.redis).await?;

        let resolver = Arc::new(Resolver::new(
            config.identity.clone(),
            http.clone(),
            coord.clone(),
        )?);
        let delegates = Arc::new(Delegates::new(
            config.delegate.clone(),
            http.clone(),
            config.nodes.clone(),
        ));
        let fleet = Arc::new(Fleet::new(config.clone(), http.clone(), coord.clone()));
        let router = Arc::new(Router::new(
            config.clone(),
            store.clone(),
            resolver.clone(),
            delegates.clone(),
            fleet.clone(),
        ));
        let forwarder = Arc::new(Forwarder::new(config.clone(), http.clone()));

        let firehose = if config.firehose.mode == crate::config::FirehoseMode::Multiplex {
            Some(Firehose::new(config.clone(), store.clone()).await?)
        } else {
            None
        };

        Ok(Arc::new(Self {
            config,
            store,
            coord,
            resolver,
            delegates,
            fleet,
            router,
            forwarder,
            firehose,
        }))
    }

    /// Starts the health monitor, firehose followers and reservation sweeper.
    pub fn spawn_background(self: &Arc<Self>) -> Vec<tokio::task::JoinHandle<()>> {
        let mut tasks = vec![self.fleet.clone().spawn_monitor()];

        if let Some(firehose) = &self.firehose {
            tasks.extend(firehose.clone().spawn());
        }

        let store = self.store.clone();
        let interval = self.config.store.sweep_interval.get();
        tasks.push(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match store.sweep_expired().await {
                    Ok(0) => {}
                    Ok(n) => tracing::debug!(swept = n, "released expired handle reservations"),
                    Err(e) => tracing::warn!(error = %e, "reservation sweep failed"),
                }
            }
        }));

        tasks
    }
}
