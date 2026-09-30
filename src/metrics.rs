//! Counters exposed as a Prometheus text exposition at `/metrics`.

use std::sync::atomic::{AtomicU64, Ordering};

pub struct Counter {
    name: &'static str,
    help: &'static str,
    value: AtomicU64,
}

impl Counter {
    const fn new(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            value: AtomicU64::new(0),
        }
    }

    pub fn incr(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add(&self, n: u64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

pub static REQUESTS: Counter = Counter::new(
    "pdsgw_requests_total",
    "XRPC requests accepted by the gateway",
);
pub static PROXIED: Counter = Counter::new(
    "pdsgw_proxied_total",
    "Requests forwarded to an upstream PDS",
);
pub static ROUTE_CACHE_HITS: Counter = Counter::new(
    "pdsgw_route_cache_hits_total",
    "Routing decisions served from cache",
);
pub static ROUTE_CACHE_MISSES: Counter = Counter::new(
    "pdsgw_route_cache_misses_total",
    "Routing decisions that required resolution",
);
pub static HANDLE_RESOLUTIONS: Counter = Counter::new(
    "pdsgw_handle_resolutions_total",
    "Handle lookups that reached DNS or HTTP",
);
pub static DID_RESOLUTIONS: Counter = Counter::new(
    "pdsgw_did_resolutions_total",
    "DID document fetches that reached the network",
);
pub static ACCOUNTS_CREATED: Counter = Counter::new(
    "pdsgw_accounts_created_total",
    "Accounts placed on an upstream PDS by the gateway",
);
pub static HANDLE_COLLISIONS: Counter = Counter::new(
    "pdsgw_handle_collisions_total",
    "Account creations refused because the handle was taken",
);
pub static UPSTREAM_ERRORS: Counter = Counter::new(
    "pdsgw_upstream_errors_total",
    "Upstream requests that failed or timed out",
);
pub static REDIS_ERRORS: Counter = Counter::new(
    "pdsgw_redis_errors_total",
    "Redis commands that failed or timed out",
);
pub static FIREHOSE_FRAMES: Counter = Counter::new(
    "pdsgw_firehose_frames_total",
    "Firehose frames relayed to subscribers",
);
pub static FIREHOSE_SUBSCRIBERS: Counter = Counter::new(
    "pdsgw_firehose_subscribers_total",
    "Firehose subscriptions accepted",
);
pub static FIREHOSE_LAGGED: Counter = Counter::new(
    "pdsgw_firehose_lagged_total",
    "Firehose subscribers disconnected for falling behind",
);

const ALL: &[&Counter] = &[
    &REQUESTS,
    &PROXIED,
    &ROUTE_CACHE_HITS,
    &ROUTE_CACHE_MISSES,
    &HANDLE_RESOLUTIONS,
    &DID_RESOLUTIONS,
    &ACCOUNTS_CREATED,
    &HANDLE_COLLISIONS,
    &UPSTREAM_ERRORS,
    &REDIS_ERRORS,
    &FIREHOSE_FRAMES,
    &FIREHOSE_SUBSCRIBERS,
    &FIREHOSE_LAGGED,
];

/// Renders every counter, plus per-node health gauges.
pub fn render(nodes: &[(String, bool, u64)]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(2048);

    for counter in ALL {
        let _ = writeln!(out, "# HELP {} {}", counter.name, counter.help);
        let _ = writeln!(out, "# TYPE {} counter", counter.name);
        let _ = writeln!(out, "{} {}", counter.name, counter.get());
    }

    out.push_str("# HELP pdsgw_node_up Whether a node passed its last health probe\n");
    out.push_str("# TYPE pdsgw_node_up gauge\n");
    for (name, up, _) in nodes {
        let _ = writeln!(out, "pdsgw_node_up{{node=\"{name}\"}} {}", u8::from(*up));
    }

    out.push_str("# HELP pdsgw_node_accounts Accounts registered on a node\n");
    out.push_str("# TYPE pdsgw_node_accounts gauge\n");
    for (name, _, accounts) in nodes {
        let _ = writeln!(out, "pdsgw_node_accounts{{node=\"{name}\"}} {accounts}");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_counters_and_node_gauges() {
        REQUESTS.add(7);
        let out = render(&[
            ("primary".to_owned(), true, 12),
            ("radxa".to_owned(), false, 3),
        ]);

        assert!(out.contains("# TYPE pdsgw_requests_total counter"));
        assert!(out.contains("pdsgw_node_up{node=\"primary\"} 1"));
        assert!(out.contains("pdsgw_node_up{node=\"radxa\"} 0"));
        assert!(out.contains("pdsgw_node_accounts{node=\"radxa\"} 3"));
        // Every declared counter is exposed.
        for counter in ALL {
            assert!(out.contains(counter.name), "missing {}", counter.name);
        }
    }
}
