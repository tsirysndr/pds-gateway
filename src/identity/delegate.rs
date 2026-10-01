//! The gateway as a handle-resolution delegate.

use std::collections::HashSet;
use std::sync::Arc;

use moka::future::Cache;
use parking_lot::Mutex;

use crate::config::{DelegateConfig, NodeConfig};
use crate::identity::{Did, Handle};

/// Where a handle claim came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub did: Did,
    /// The node that answered, when the claim came from a fan-out.
    pub node: Option<String>,
}

pub struct Delegates {
    config: DelegateConfig,
    http: reqwest::Client,
    nodes: Vec<NodeConfig>,
    claims: Cache<String, Claim>,
    in_flight: Arc<Mutex<HashSet<String>>>,
}

pub const HOP_HEADER: &str = "x-pdsgw-delegate-hop";

impl Delegates {
    pub fn new(config: DelegateConfig, http: reqwest::Client, nodes: Vec<NodeConfig>) -> Self {
        let claims = Cache::builder()
            .max_capacity(50_000)
            .time_to_live(config.cache_ttl.get())
            .build();

        Self {
            config,
            http,
            nodes,
            claims,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// True while a fan-out for `handle` is in progress, which means an incoming
    /// ask for it must be answered from local state only.
    pub fn is_reentrant(&self, handle: &Handle) -> bool {
        self.in_flight.lock().contains(handle.as_str())
    }

    pub async fn resolve(&self, handle: &Handle, cached: bool) -> Option<Claim> {
        if !self.config.enabled || !self.config.fan_out {
            return None;
        }

        let key = handle.as_str().to_owned();

        if cached && let Some(hit) = self.claims.get(&key).await {
            return Some(hit);
        }

        if !self.begin(&key) {
            tracing::debug!(handle = %handle, "delegate ask already in flight; not recursing");
            return None;
        }
        let _guard = InFlight {
            set: self.in_flight.clone(),
            key: key.clone(),
        };

        let claim = self.ask_all(handle).await?;
        // Only answers are cached; a failure is retried next time.
        self.claims.insert(key, claim.clone()).await;
        Some(claim)
    }

    fn begin(&self, key: &str) -> bool {
        self.in_flight.lock().insert(key.to_owned())
    }

    async fn ask_all(&self, handle: &Handle) -> Option<Claim> {
        let mut asks = futures::stream::FuturesUnordered::new();
        for node in &self.nodes {
            asks.push(self.ask(node, handle));
        }

        use futures::StreamExt;
        while let Some(answer) = asks.next().await {
            if let Some(claim) = answer {
                return Some(claim);
            }
        }
        None
    }

    async fn ask(&self, node: &NodeConfig, handle: &Handle) -> Option<Claim> {
        let url = format!(
            "{}/xrpc/com.atproto.identity.resolveHandle",
            node.url.as_str().trim_end_matches('/')
        );

        let request = self
            .http
            .get(&url)
            .query(&[("handle", handle.as_str())])
            .header(HOP_HEADER, "1")
            .timeout(self.config.ask_timeout.get());

        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                tracing::debug!(node = %node.name, handle = %handle, error = %e, "delegate ask failed");
                return None;
            }
        };

        // Anything other than a 200 means this node does not claim the handle.
        if response.status() != reqwest::StatusCode::OK {
            return None;
        }

        let body: serde_json::Value = response.json().await.ok()?;
        let did = Did::parse(body.get("did")?.as_str()?).ok()?;

        tracing::debug!(node = %node.name, handle = %handle, did = %did, "delegate claim");
        Some(Claim {
            did,
            node: Some(node.name.clone()),
        })
    }

    pub async fn invalidate(&self, handle: &Handle) {
        self.claims.invalidate(handle.as_str()).await;
    }

    pub async fn remember(&self, handle: &Handle, claim: Claim) {
        self.claims.insert(handle.as_str().to_owned(), claim).await;
    }
}

/// Clears the in-flight marker even if the fan-out panics or is cancelled.
struct InFlight {
    set: Arc<Mutex<HashSet<String>>>,
    key: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.set.lock().remove(&self.key);
    }
}

/// Whether a delegate ask arriving with this header should be answered without
/// fanning out again.
pub fn is_hop(headers: &axum::http::HeaderMap) -> bool {
    headers.contains_key(HOP_HEADER)
}

/// atoll accepts only these DID shapes from a delegate; matching it here keeps
/// the gateway from answering with something the caller would discard.
pub fn is_delegate_safe(did: &Did) -> bool {
    match did.method() {
        "plc" => {
            let id = did.identifier();
            id.len() == 24
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        }
        "web" => {
            let id = did.identifier();
            (1..=250).contains(&id.len())
                && id.bytes().all(|b| {
                    b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || matches!(b, b'.' | b':' | b'%' | b'-')
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use url::Url;

    fn nodes() -> Vec<NodeConfig> {
        vec![NodeConfig {
            name: "radxa".into(),
            url: Url::parse("http://127.0.0.1:1").unwrap(),
            public_host: Some("radxa.rocksky.social".into()),
            did: None,
            weight: 1,
            accepts_signups: true,
            max_accounts: None,
            signin_path: None,
        }]
    }

    fn delegates(config: DelegateConfig) -> Delegates {
        Delegates::new(config, reqwest::Client::new(), nodes())
    }

    #[test]
    fn accepts_the_did_shapes_a_caller_will_keep() {
        // did:plc is exactly 24 base32-sortable characters.
        assert!(is_delegate_safe(
            &Did::parse("did:plc:7iza6de2dwap2sbkpav7c6c6").unwrap()
        ));
        assert!(is_delegate_safe(
            &Did::parse("did:web:radxa.rocksky.social").unwrap()
        ));
        assert!(is_delegate_safe(
            &Did::parse("did:web:localhost%3a2583").unwrap()
        ));
        // atoll's delegate regex is lowercase-only, so an uppercase escape
        // would be discarded by the caller.
        assert!(!is_delegate_safe(
            &Did::parse("did:web:localhost%3A2583").unwrap()
        ));

        // Too short, wrong alphabet, or an unusable method.
        assert!(!is_delegate_safe(&Did::parse("did:plc:abc").unwrap()));
        assert!(!is_delegate_safe(
            &Did::parse("did:plc:7IZA6DE2DWAP2SBKPAV7C6C6").unwrap()
        ));
        assert!(!is_delegate_safe(
            &Did::parse("did:plc:1iza6de2dwap2sbkpav7c6c6").unwrap()
        ));
        assert!(!is_delegate_safe(&Did::parse("did:key:zabc").unwrap()));
    }

    #[tokio::test]
    async fn a_disabled_delegate_never_asks() {
        let d = delegates(DelegateConfig {
            enabled: false,
            ..DelegateConfig::default()
        });
        let handle = Handle::parse("alice.rocksky.social").unwrap();
        assert!(d.resolve(&handle, true).await.is_none());
    }

    #[tokio::test]
    async fn fan_out_can_be_turned_off_independently() {
        let d = delegates(DelegateConfig {
            fan_out: false,
            ..DelegateConfig::default()
        });
        let handle = Handle::parse("alice.rocksky.social").unwrap();
        assert!(d.resolve(&handle, true).await.is_none());
    }

    #[tokio::test]
    async fn an_unreachable_node_leaves_the_handle_unclaimed() {
        let d = delegates(DelegateConfig {
            ask_timeout: Duration::from_millis(150).into(),
            ..DelegateConfig::default()
        });
        let handle = Handle::parse("alice.rocksky.social").unwrap();
        // A node that cannot be reached must not make the name look taken.
        assert!(d.resolve(&handle, false).await.is_none());
    }

    #[tokio::test]
    async fn a_remembered_claim_is_served_from_cache() {
        let d = delegates(DelegateConfig::default());
        let handle = Handle::parse("alice.rocksky.social").unwrap();
        let claim = Claim {
            did: Did::parse("did:plc:7iza6de2dwap2sbkpav7c6c6").unwrap(),
            node: Some("radxa".into()),
        };

        d.remember(&handle, claim.clone()).await;
        assert_eq!(d.resolve(&handle, true).await, Some(claim));

        d.invalidate(&handle).await;
        // With the cache cleared the only node is unreachable, so no claim.
        assert!(d.resolve(&handle, true).await.is_none());
    }

    #[tokio::test]
    async fn the_in_flight_guard_blocks_re_entry_and_then_clears() {
        let d = delegates(DelegateConfig::default());
        let handle = Handle::parse("alice.rocksky.social").unwrap();

        assert!(!d.is_reentrant(&handle));
        assert!(d.begin(handle.as_str()));
        assert!(d.is_reentrant(&handle));
        // A second ask for the same handle must not fan out again.
        assert!(d.resolve(&handle, false).await.is_none());

        drop(InFlight {
            set: d.in_flight.clone(),
            key: handle.as_str().to_owned(),
        });
        assert!(!d.is_reentrant(&handle));
    }

    #[test]
    fn detects_the_hop_header() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!is_hop(&headers));
        headers.insert(HOP_HEADER, "1".parse().unwrap());
        assert!(is_hop(&headers));
    }
}
