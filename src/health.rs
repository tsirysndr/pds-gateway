//! Per-node health tracking and the placement rotation counter.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::config::{Config, NodeConfig};
use crate::registry::coord::Coordinator;

#[derive(Debug, Clone)]
pub struct NodeHealth {
    pub up: bool,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub last_checked: Option<Instant>,
    pub last_latency: Option<Duration>,
    pub last_error: Option<String>,
}

impl Default for NodeHealth {
    fn default() -> Self {
        Self {
            // Assume reachable until a probe says otherwise, so a restart does
            // not reject traffic for one probe interval.
            up: true,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_checked: None,
            last_latency: None,
            last_error: None,
        }
    }
}

pub struct Fleet {
    config: Arc<Config>,
    http: reqwest::Client,
    coord: Coordinator,
    state: RwLock<HashMap<String, NodeHealth>>,
}

impl Fleet {
    pub fn new(config: Arc<Config>, http: reqwest::Client, coord: Coordinator) -> Self {
        let state = config
            .nodes
            .iter()
            .map(|n| (n.name.clone(), NodeHealth::default()))
            .collect();

        Self {
            config,
            http,
            coord,
            state: RwLock::new(state),
        }
    }

    pub fn is_up(&self, node: &str) -> bool {
        self.state.read().get(node).is_some_and(|h| h.up)
    }

    /// Whether a request may be sent to `node`. Reads can still succeed on a node
    /// that failed its probe, so this is configurable.
    pub fn is_routable(&self, node: &str) -> bool {
        self.config.health.route_to_unhealthy || self.is_up(node)
    }

    pub fn snapshot(&self) -> Vec<(String, NodeHealth)> {
        let state = self.state.read();
        self.config
            .nodes
            .iter()
            .map(|n| {
                (
                    n.name.clone(),
                    state.get(&n.name).cloned().unwrap_or_default(),
                )
            })
            .collect()
    }

    pub async fn next_rotation(&self) -> u64 {
        self.coord.next_rotation().await
    }

    /// Nodes to try for a broadcast, healthy ones first.
    pub fn broadcast_order(&self, preferred: Option<&str>) -> Vec<String> {
        let state = self.state.read();
        let mut nodes: Vec<&NodeConfig> = self.config.nodes.iter().collect();
        nodes.sort_by_key(|n| {
            let up = state.get(&n.name).is_some_and(|h| h.up);
            (Some(n.name.as_str()) != preferred, !up, n.name.clone())
        });
        nodes.into_iter().map(|n| n.name.clone()).collect()
    }

    async fn probe(&self, node: &NodeConfig) -> Result<Duration, String> {
        let url = format!(
            "{}{}",
            node.url.as_str().trim_end_matches('/'),
            self.config.health.probe_path
        );
        let started = Instant::now();

        let response = self
            .http
            .get(&url)
            .timeout(self.config.health.timeout.get())
            .header("host", node.effective_public_host())
            .header("x-forwarded-proto", self.config.server.public_url.scheme())
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if response.status().is_success() {
            Ok(started.elapsed())
        } else {
            Err(format!("probe returned {}", response.status()))
        }
    }

    async fn check_all(&self) {
        let probes = self.config.nodes.iter().map(|node| async move {
            let result = self.probe(node).await;
            (node.name.clone(), result)
        });
        let results = futures::future::join_all(probes).await;

        for (name, result) in results {
            let mut state = self.state.write();
            let health = state.entry(name.clone()).or_default();
            health.last_checked = Some(Instant::now());

            match result {
                Ok(latency) => {
                    health.consecutive_failures = 0;
                    health.consecutive_successes += 1;
                    health.last_latency = Some(latency);
                    health.last_error = None;

                    if !health.up
                        && health.consecutive_successes >= self.config.health.success_threshold
                    {
                        health.up = true;
                        tracing::info!(node = %name, ?latency, "node is back up");
                    }
                }
                Err(error) => {
                    health.consecutive_successes = 0;
                    health.consecutive_failures += 1;
                    health.last_error = Some(error.clone());

                    if health.up
                        && health.consecutive_failures >= self.config.health.failure_threshold
                    {
                        health.up = false;
                        tracing::warn!(
                            node = %name,
                            failures = health.consecutive_failures,
                            %error,
                            "node marked down"
                        );
                    } else if health.up {
                        tracing::debug!(node = %name, %error, "node probe failed");
                    }
                }
            }
        }
    }

    pub fn spawn_monitor(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.config.health.interval.get());
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                self.check_all().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GatewayConfig, HealthConfig};
    use url::Url;

    fn config(route_to_unhealthy: bool) -> Arc<Config> {
        let node = |name: &str, port: u16| NodeConfig {
            name: name.into(),
            url: Url::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            public_host: Some(format!("{name}.rocksky.social")),
            did: None,
            weight: 1,
            accepts_signups: true,
            max_accounts: None,
        };

        Arc::new(Config {
            gateway: GatewayConfig {
                handle_domains: vec!["rocksky.social".into()],
                default_node: Some("primary".into()),
                ..GatewayConfig::default()
            },
            health: HealthConfig {
                failure_threshold: 2,
                success_threshold: 2,
                timeout: Duration::from_millis(100).into(),
                route_to_unhealthy,
                ..HealthConfig::default()
            },
            nodes: vec![node("primary", 1), node("radxa", 2)],
            ..Config::default()
        })
    }

    async fn fleet(route_to_unhealthy: bool) -> Arc<Fleet> {
        let coord = Coordinator::connect(&Default::default()).await.unwrap();
        Arc::new(Fleet::new(
            config(route_to_unhealthy),
            reqwest::Client::new(),
            coord,
        ))
    }

    #[tokio::test]
    async fn nodes_start_assumed_up() {
        let fleet = fleet(false).await;
        assert!(fleet.is_up("primary"));
        assert!(fleet.is_routable("primary"));
        assert!(!fleet.is_up("unknown-node"));
    }

    #[tokio::test]
    async fn a_node_goes_down_only_after_the_threshold() {
        let fleet = fleet(false).await;
        // Nothing is listening on these ports, so every probe fails.
        fleet.check_all().await;
        assert!(fleet.is_up("primary"), "one failure must not flip the node");

        fleet.check_all().await;
        assert!(!fleet.is_up("primary"), "two failures should mark it down");
        assert!(!fleet.is_routable("primary"));

        let snapshot: HashMap<_, _> = fleet.snapshot().into_iter().collect();
        assert_eq!(snapshot["primary"].consecutive_failures, 2);
        assert!(snapshot["primary"].last_error.is_some());
    }

    #[tokio::test]
    async fn an_unhealthy_node_stays_routable_when_configured() {
        let fleet = fleet(true).await;
        fleet.check_all().await;
        fleet.check_all().await;

        assert!(!fleet.is_up("primary"));
        assert!(
            fleet.is_routable("primary"),
            "reads should still be attempted"
        );
    }

    #[tokio::test]
    async fn broadcast_prefers_the_preferred_then_healthy_nodes() {
        let fleet = fleet(false).await;
        let order = fleet.broadcast_order(Some("radxa"));
        assert_eq!(order[0], "radxa");

        fleet.check_all().await;
        fleet.check_all().await;
        // With every node down the order is still stable and complete.
        let order = fleet.broadcast_order(None);
        assert_eq!(order.len(), 2);
        assert_eq!(order, vec!["primary".to_owned(), "radxa".to_owned()]);
    }

    #[tokio::test]
    async fn the_rotation_counter_advances() {
        let fleet = fleet(false).await;
        let first = fleet.next_rotation().await;
        assert_eq!(fleet.next_rotation().await, first + 1);
    }
}
