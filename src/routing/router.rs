//! Turning a request into a target node.

use std::sync::Arc;

use serde_json::Value;

use crate::config::Config;
use crate::error::{GatewayError, Result};
use crate::health::Fleet;
use crate::identity::{Claim, Delegates, Did, Handle, Resolver};
use crate::registry::store::Store;
use crate::routing::auth::TokenClaims;
use crate::routing::lexicon::{self, Handling, Source};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Did(Did),
    Handle(Handle),
    /// A login identifier that is neither: an email address.
    Opaque(String),
}

impl Subject {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if raw.starts_with("did:") {
            return Did::parse(raw).ok().map(Self::Did);
        }
        if let Ok(handle) = Handle::parse(raw) {
            return Some(Self::Handle(handle));
        }
        Some(Self::Opaque(raw.to_owned()))
    }

    pub fn as_key(&self) -> &str {
        match self {
            Self::Did(did) => did.as_str(),
            Self::Handle(handle) => handle.as_str(),
            Self::Opaque(raw) => raw,
        }
    }
}

impl std::fmt::Display for Subject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_key())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Forward to one node.
    Node { name: String, why: Why },
    /// Query every node and merge.
    Fanout,
    /// Try each node until one accepts.
    Broadcast { preferred: Option<String> },
    /// The gateway answers.
    Local,
}

/// How far the gateway may go to answer a handle lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// Only what the gateway already recorded. Used when answering a node that
    /// is itself asking us, so the question cannot bounce back.
    RegistryOnly,
    /// Registry, then the nodes, honouring the delegate cache.
    Cached,
    /// Registry, then the nodes, ignoring the cache. Required before allocating
    /// a handle: a cached answer can only say "taken".
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Registry,
    Delegate,
    DidDocument,
    TokenAudience,
    Hint,
    ProxyHeader,
    NodeParam,
    Default,
}

pub struct Router {
    config: Arc<Config>,
    store: Arc<Store>,
    resolver: Arc<Resolver>,
    delegates: Arc<Delegates>,
    fleet: Arc<Fleet>,
}

impl Router {
    pub fn new(
        config: Arc<Config>,
        store: Arc<Store>,
        resolver: Arc<Resolver>,
        delegates: Arc<Delegates>,
        fleet: Arc<Fleet>,
    ) -> Self {
        Self {
            config,
            store,
            resolver,
            delegates,
            fleet,
        }
    }

    pub fn default_node(&self) -> String {
        self.config
            .gateway
            .default_node
            .clone()
            .unwrap_or_else(|| self.config.nodes[0].name.clone())
    }

    /// Decides where an XRPC call goes.
    pub async fn route(
        &self,
        nsid: &str,
        query: &[(String, String)],
        body: Option<&Value>,
        auth: Option<&TokenClaims>,
        headers: &axum::http::HeaderMap,
    ) -> Result<Decision> {
        // An explicit ?node= wins, so operators can pin a call while debugging.
        if let Some(node) = param(query, "node")
            && self.config.node(&node).is_some()
        {
            return Ok(Decision::Node {
                name: node,
                why: Why::NodeParam,
            });
        }

        let handling = lexicon::classify(nsid);
        match &handling {
            Handling::Local => Ok(Decision::Local),
            Handling::Fanout => Ok(Decision::Fanout),
            Handling::Broadcast { sources } => {
                let subject = self.find_subject(sources, query, body, auth);
                let preferred = match &subject {
                    Some(subject) => self.locate(subject).await?,
                    None => None,
                };
                Ok(Decision::Broadcast { preferred })
            }
            Handling::Proxy { sources } => {
                if let Some(node) = self.node_for_proxy_header(headers) {
                    return Ok(Decision::Node {
                        name: node,
                        why: Why::ProxyHeader,
                    });
                }

                if let Some(subject) = self.find_subject(sources, query, body, auth)
                    && let Some(name) = self.locate(&subject).await?
                {
                    return Ok(Decision::Node {
                        name,
                        why: self.last_why(),
                    });
                }

                // A session token names its issuing PDS in `aud`.
                if let Some(name) = auth.and_then(|a| self.node_for_audience(a)) {
                    return Ok(Decision::Node {
                        name,
                        why: Why::TokenAudience,
                    });
                }

                Ok(Decision::Node {
                    name: self.default_node(),
                    why: Why::Default,
                })
            }
        }
    }

    fn last_why(&self) -> Why {
        Why::Registry
    }

    fn find_subject(
        &self,
        sources: &[Source],
        query: &[(String, String)],
        body: Option<&Value>,
        auth: Option<&TokenClaims>,
    ) -> Option<Subject> {
        for source in sources {
            let raw = match source {
                Source::Query(key) => param(query, key),
                Source::Body(key) => body
                    .and_then(|b| b.get(*key))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                Source::BodyPath(path) => body.and_then(|b| {
                    path.split('.')
                        .try_fold(b, |value, key| value.get(key))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                }),
                Source::AuthSubject => auth.and_then(|a| a.sub.clone()),
            };

            // `dids` on getAccountInfos is a repeated parameter; the first is
            // enough to pick a node.
            let raw = raw.map(|r| r.split(',').next().unwrap_or(&r).trim().to_owned());

            if let Some(subject) = raw.as_deref().and_then(Subject::parse) {
                return Some(subject);
            }
        }
        None
    }

    /// Finds the node hosting a subject: registry, then hint, then the network.
    pub async fn locate(&self, subject: &Subject) -> Result<Option<String>> {
        match subject {
            Subject::Did(did) => self.locate_did(did).await,
            Subject::Handle(handle) => self.locate_handle(handle).await,
            Subject::Opaque(raw) => Ok(self.store.hint(raw).await?),
        }
    }

    async fn locate_did(&self, did: &Did) -> Result<Option<String>> {
        if let Some(account) = self.store.account_by_did(did.as_str()).await? {
            crate::metrics::ROUTE_CACHE_HITS.incr();
            return Ok(Some(account.node));
        }
        crate::metrics::ROUTE_CACHE_MISSES.incr();

        // A node's own DID routes to that node.
        if let Some(node) = self
            .config
            .nodes
            .iter()
            .find(|n| n.did.as_deref() == Some(did.as_str()))
        {
            return Ok(Some(node.name.clone()));
        }

        let Some(document) = self.resolver.resolve_did(did).await? else {
            return Ok(None);
        };
        let Some(host) = document.pds_host() else {
            return Ok(None);
        };
        let Some(node) = self.node_for_host(&host) else {
            // The DID lives on a PDS outside this fleet; the default node knows
            // how to federate to it.
            return Ok(None);
        };

        // Learn it, so the next request is a single SQLite read.
        let handle = document
            .claimed_handle()
            .and_then(|h| Handle::parse(h).ok())
            .map(Handle::into_string)
            .unwrap_or_default();
        if !handle.is_empty() {
            self.store
                .upsert_account(did.as_str(), &handle, &node)
                .await?;
        }

        Ok(Some(node))
    }

    async fn locate_handle(&self, handle: &Handle) -> Result<Option<String>> {
        if let Some(account) = self.store.account_by_handle(handle.as_str()).await? {
            crate::metrics::ROUTE_CACHE_HITS.incr();
            return Ok(Some(account.node));
        }
        crate::metrics::ROUTE_CACHE_MISSES.incr();

        // Inside the namespace the gateway owns, the nodes themselves are the
        // authority for handles it has not recorded yet.
        if self.config.owns_handle(handle.as_str())
            && let Some(claim) = self.delegates.resolve(handle, true).await
        {
            self.remember_claim(handle, &claim).await?;
            return Ok(claim.node);
        }

        let Some(did) = self.resolver.resolve_handle(handle).await? else {
            return Ok(None);
        };
        self.locate_did(&did).await
    }

    async fn remember_claim(&self, handle: &Handle, claim: &Claim) -> Result<()> {
        if let Some(node) = &claim.node {
            self.store
                .upsert_account(claim.did.as_str(), handle.as_str(), node)
                .await?;
        }
        Ok(())
    }

    /// Resolves a handle for the delegate endpoint: registry first, then ask the
    /// nodes. `live` skips the cache, which the allocation path requires.
    pub async fn resolve_owned_handle(
        &self,
        handle: &Handle,
        lookup: Lookup,
    ) -> Result<Option<Claim>> {
        if let Some(account) = self.store.account_by_handle(handle.as_str()).await? {
            return Ok(Some(Claim {
                did: Did::parse(&account.did)?,
                node: Some(account.node),
            }));
        }

        // A node asking us must not send us back to that node.
        if lookup == Lookup::RegistryOnly || self.delegates.is_reentrant(handle) {
            return Ok(None);
        }

        match self
            .delegates
            .resolve(handle, lookup == Lookup::Cached)
            .await
        {
            Some(claim) => {
                self.remember_claim(handle, &claim).await?;
                Ok(Some(claim))
            }
            None => Ok(None),
        }
    }

    fn node_for_host(&self, host: &str) -> Option<String> {
        let host = host.to_ascii_lowercase();
        self.config
            .nodes
            .iter()
            .find(|n| n.effective_public_host() == host || n.url.host_str() == Some(host.as_str()))
            .map(|n| n.name.clone())
    }

    fn node_for_audience(&self, claims: &TokenClaims) -> Option<String> {
        let aud = claims.aud.as_deref()?;
        if let Some(node) = self
            .config
            .nodes
            .iter()
            .find(|n| n.did.as_deref() == Some(aud))
        {
            return Some(node.name.clone());
        }
        // `did:web:<host>` names the node by hostname.
        Did::parse(aud)
            .ok()
            .and_then(|did| did.web_host())
            .and_then(|host| self.node_for_host(&host))
    }

    /// `atproto-proxy: <did>#<service>` asks the PDS to forward somewhere. Only
    /// honoured when it names one of our own nodes; foreign targets are left for
    /// the upstream PDS to handle.
    fn node_for_proxy_header(&self, headers: &axum::http::HeaderMap) -> Option<String> {
        if !self.config.gateway.honor_proxy_header {
            return None;
        }
        let value = headers.get("atproto-proxy")?.to_str().ok()?;
        let did = value.split('#').next()?;
        self.config
            .nodes
            .iter()
            .find(|n| n.did.as_deref() == Some(did))
            .map(|n| n.name.clone())
    }

    /// Picks a node for a new account.
    pub async fn place(&self) -> Result<String> {
        use crate::config::Placement;

        let counts: std::collections::HashMap<String, u64> =
            self.store.counts_by_node().await?.into_iter().collect();

        let eligible: Vec<_> = self
            .config
            .nodes
            .iter()
            .filter(|n| n.accepts_signups)
            .filter(|n| self.fleet.is_up(&n.name))
            .filter(|n| {
                n.max_accounts
                    .is_none_or(|max| counts.get(&n.name).copied().unwrap_or(0) < max)
            })
            .collect();

        if eligible.is_empty() {
            return Err(GatewayError::NoNodeAvailable(
                "every node is down, full, or closed to signups".to_owned(),
            ));
        }

        let chosen = match self.config.gateway.placement {
            Placement::Pinned => {
                let default = self.default_node();
                eligible
                    .iter()
                    .find(|n| n.name == default)
                    .copied()
                    .unwrap_or(eligible[0])
            }
            Placement::LeastAccounts => eligible
                .iter()
                .copied()
                .min_by_key(|n| counts.get(&n.name).copied().unwrap_or(0))
                .expect("eligible is non-empty"),
            Placement::RoundRobin => {
                let tick = self.fleet.next_rotation().await;
                eligible[(tick as usize) % eligible.len()]
            }
            Placement::Weighted => {
                let total: u64 = eligible.iter().map(|n| u64::from(n.weight)).sum();
                if total == 0 {
                    eligible[0]
                } else {
                    let tick = self.fleet.next_rotation().await % total;
                    let mut acc = 0;
                    eligible
                        .iter()
                        .copied()
                        .find(|n| {
                            acc += u64::from(n.weight);
                            tick < acc
                        })
                        .unwrap_or(eligible[0])
                }
            }
        };

        Ok(chosen.name.clone())
    }
}

fn param(query: &[(String, String)], key: &str) -> Option<String> {
    query
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.trim().is_empty())
}
