//! Handle and DID resolution with a two-tier cache.

use std::sync::Arc;
use std::time::Duration;

use hickory_resolver::TokioAsyncResolver;
use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::error::ResolveErrorKind;
use moka::future::Cache;
use serde::Deserialize;
use url::Url;

use crate::config::IdentityConfig;
use crate::error::GatewayError;
use crate::identity::{Did, Handle};
use crate::registry::coord::Coordinator;

const NS_HANDLE: &str = "h2d";
const NS_DOC: &str = "doc";
/// Sentinel stored for a handle that resolved to nothing, so the negative
/// result is cached rather than re-queried on every request.
const NEGATIVE: &str = "\u{0}none";

#[derive(Debug, Clone, Deserialize)]
pub struct DidService {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub service_type: String,
    #[serde(default, rename = "serviceEndpoint")]
    pub service_endpoint: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DidDocument {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "alsoKnownAs")]
    pub also_known_as: Vec<String>,
    #[serde(default)]
    pub service: Vec<DidService>,
}

impl DidDocument {
    /// The `AtprotoPersonalDataServer` endpoint, matched on the service `id`
    /// suffix the spec fixes (`#atproto_pds`) and falling back to the type.
    pub fn pds_endpoint(&self) -> Option<&str> {
        self.service
            .iter()
            .find(|s| s.id.ends_with("#atproto_pds"))
            .or_else(|| {
                self.service
                    .iter()
                    .find(|s| s.service_type == "AtprotoPersonalDataServer")
            })
            .map(|s| s.service_endpoint.as_str())
            .filter(|e| !e.is_empty())
    }

    /// Hostname of the PDS endpoint, which is what node routing matches on.
    pub fn pds_host(&self) -> Option<String> {
        let endpoint = self.pds_endpoint()?;
        Url::parse(endpoint)
            .ok()?
            .host_str()
            .map(|h| h.to_ascii_lowercase())
    }

    pub fn claimed_handle(&self) -> Option<&str> {
        self.also_known_as
            .iter()
            .find_map(|aka| aka.strip_prefix("at://"))
            .filter(|h| !h.is_empty())
    }
}

pub struct Resolver {
    config: IdentityConfig,
    http: reqwest::Client,
    dns: Option<TokioAsyncResolver>,
    coord: Coordinator,
    handles: Cache<String, Option<Did>>,
    documents: Cache<String, Option<Arc<DidDocument>>>,
}

impl Resolver {
    pub fn new(
        config: IdentityConfig,
        http: reqwest::Client,
        coord: Coordinator,
    ) -> anyhow::Result<Self> {
        let dns = if config.dns_resolution {
            Some(build_dns_resolver(&config)?)
        } else {
            None
        };

        // Negative entries expire sooner than positive ones, so a handle that
        // has just been published becomes visible quickly.
        let handles = Cache::builder()
            .max_capacity(config.cache_capacity)
            .expire_after(HandleExpiry {
                hit: config.handle_cache_ttl.get(),
                miss: config.handle_negative_cache_ttl.get(),
            })
            .build();

        let documents = Cache::builder()
            .max_capacity(config.cache_capacity)
            .time_to_live(config.did_cache_ttl.get())
            .build();

        Ok(Self {
            config,
            http,
            dns,
            coord,
            handles,
            documents,
        })
    }

    /// Resolves a handle to its DID, or `None` when the handle is unclaimed.
    pub async fn resolve_handle(&self, handle: &Handle) -> Result<Option<Did>, GatewayError> {
        let key = handle.as_str().to_owned();

        self.handles
            .try_get_with(key.clone(), async {
                if let Some(cached) = self.coord.cache_get(NS_HANDLE, &key).await {
                    let hit = if cached == NEGATIVE {
                        None
                    } else {
                        Did::parse(&cached).ok()
                    };
                    // A malformed shared entry is treated as a miss.
                    if hit.is_some() || cached == NEGATIVE {
                        return Ok::<_, GatewayError>(hit);
                    }
                }

                crate::metrics::HANDLE_RESOLUTIONS.incr();
                let resolved = self.resolve_handle_uncached(handle).await;

                let (payload, ttl) = match &resolved {
                    Some(did) => (did.as_str().to_owned(), self.config.handle_cache_ttl.get()),
                    None => (
                        NEGATIVE.to_owned(),
                        self.config.handle_negative_cache_ttl.get(),
                    ),
                };
                self.coord.cache_put(NS_HANDLE, &key, &payload, ttl).await;

                Ok(resolved)
            })
            .await
            .map_err(|e: Arc<GatewayError>| GatewayError::internal(e))
    }

    /// DNS first (authoritative and cheap), then `.well-known` over HTTPS.
    async fn resolve_handle_uncached(&self, handle: &Handle) -> Option<Did> {
        if self.dns.is_some() {
            match self.resolve_handle_dns(handle).await {
                Ok(Some(did)) => return Some(did),
                Ok(None) => {}
                Err(e) => tracing::debug!(handle = %handle, error = %e, "handle DNS lookup failed"),
            }
        }

        if self.config.well_known_resolution {
            match self.resolve_handle_well_known(handle).await {
                Ok(Some(did)) => return Some(did),
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!(handle = %handle, error = %e, "handle well-known lookup failed")
                }
            }
        }

        None
    }

    async fn resolve_handle_dns(&self, handle: &Handle) -> anyhow::Result<Option<Did>> {
        let Some(dns) = &self.dns else {
            return Ok(None);
        };
        let name = format!("_atproto.{handle}");
        let lookup = match dns.txt_lookup(&name).await {
            Ok(lookup) => lookup,
            // NXDOMAIN and friends mean "not claimed here", not a failure.
            Err(e) if matches!(e.kind(), ResolveErrorKind::NoRecordsFound { .. }) => {
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };

        let mut found = None;
        for record in lookup.iter() {
            for chunk in record.txt_data() {
                let text = String::from_utf8_lossy(chunk);
                if let Some(value) = text.trim().strip_prefix("did=") {
                    let did = Did::parse(value.trim())?;
                    // Two competing TXT records are ambiguous; refuse rather
                    // than pick one arbitrarily.
                    if found.as_ref().is_some_and(|f| f != &did) {
                        anyhow::bail!("`{name}` publishes more than one DID");
                    }
                    found = Some(did);
                }
            }
        }
        Ok(found)
    }

    async fn resolve_handle_well_known(&self, handle: &Handle) -> anyhow::Result<Option<Did>> {
        let url = format!("https://{handle}/.well-known/atproto-did");
        let response = self.http.get(&url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            anyhow::bail!("{url} returned {}", response.status());
        }

        // Cap the read: this endpoint should be a single line.
        let body = response.text().await?;
        if body.len() > 4096 {
            anyhow::bail!("{url} returned an implausibly large body");
        }
        Ok(Some(Did::parse(body.trim())?))
    }

    /// Fetches and caches a DID document. `None` means the DID does not exist.
    pub async fn resolve_did(&self, did: &Did) -> Result<Option<Arc<DidDocument>>, GatewayError> {
        let key = did.as_str().to_owned();

        self.documents
            .try_get_with(key.clone(), async {
                if let Some(cached) = self.coord.cache_get(NS_DOC, &key).await {
                    if cached == NEGATIVE {
                        return Ok::<_, GatewayError>(None);
                    }
                    if let Ok(doc) = serde_json::from_str::<DidDocument>(&cached) {
                        return Ok(Some(Arc::new(doc)));
                    }
                }

                crate::metrics::DID_RESOLUTIONS.incr();
                let fetched = self.fetch_did_document(did).await.map_err(|e| {
                    GatewayError::UnresolvableSubject {
                        subject: did.to_string(),
                        reason: e.to_string(),
                    }
                })?;

                match &fetched {
                    Some(doc) => {
                        if let Ok(json) = serde_json::to_string(&SerializableDoc::from(&**doc)) {
                            self.coord
                                .cache_put(NS_DOC, &key, &json, self.config.did_cache_ttl.get())
                                .await;
                        }
                    }
                    None => {
                        self.coord
                            .cache_put(
                                NS_DOC,
                                &key,
                                NEGATIVE,
                                self.config.handle_negative_cache_ttl.get(),
                            )
                            .await;
                    }
                }

                Ok(fetched)
            })
            .await
            .map_err(|e: Arc<GatewayError>| match &*e {
                GatewayError::UnresolvableSubject { subject, reason } => {
                    GatewayError::UnresolvableSubject {
                        subject: subject.clone(),
                        reason: reason.clone(),
                    }
                }
                other => GatewayError::internal(other),
            })
    }

    async fn fetch_did_document(&self, did: &Did) -> anyhow::Result<Option<Arc<DidDocument>>> {
        let url = match did.method() {
            "plc" => format!(
                "{}/{}",
                self.config.plc_directory_url.as_str().trim_end_matches('/'),
                did
            ),
            "web" => {
                let host = did
                    .web_host()
                    .ok_or_else(|| anyhow::anyhow!("did:web with no host"))?;
                // Only loopback may be plain HTTP, matching PDS behaviour in
                // local development.
                let scheme = if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
                    "http"
                } else {
                    "https"
                };
                format!("{scheme}://{host}/.well-known/did.json")
            }
            other => anyhow::bail!("unsupported DID method `{other}`"),
        };

        let response = self.http.get(&url).send().await?;
        if matches!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) {
            return Ok(None);
        }
        if !response.status().is_success() {
            anyhow::bail!("{url} returned {}", response.status());
        }

        let document: DidDocument = response.json().await?;
        if document.id != did.as_str() {
            anyhow::bail!(
                "{url} returned a document for `{}`, not `{did}`",
                document.id
            );
        }
        Ok(Some(Arc::new(document)))
    }

    /// Drops every cached entry for a subject. Called after the gateway itself
    /// changes an account, so the next request sees the new state immediately.
    pub async fn invalidate_did(&self, did: &Did) {
        self.documents.invalidate(did.as_str()).await;
        self.coord.cache_del(NS_DOC, did.as_str()).await;
    }

    pub async fn invalidate_handle(&self, handle: &Handle) {
        self.handles.invalidate(handle.as_str()).await;
        self.coord.cache_del(NS_HANDLE, handle.as_str()).await;
    }

    pub fn cache_sizes(&self) -> (u64, u64) {
        (self.handles.entry_count(), self.documents.entry_count())
    }
}

/// `moka` expiry that gives negative results a shorter life than hits.
struct HandleExpiry {
    hit: Duration,
    miss: Duration,
}

impl moka::Expiry<String, Option<Did>> for HandleExpiry {
    fn expire_after_create(
        &self,
        _key: &String,
        value: &Option<Did>,
        _now: std::time::Instant,
    ) -> Option<Duration> {
        Some(if value.is_some() { self.hit } else { self.miss })
    }
}

/// `DidDocument` is deserialize-only; this mirrors it for the L2 cache.
#[derive(serde::Serialize)]
struct SerializableDoc<'a> {
    id: &'a str,
    #[serde(rename = "alsoKnownAs")]
    also_known_as: &'a [String],
    service: Vec<SerializableService<'a>>,
}

#[derive(serde::Serialize)]
struct SerializableService<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    service_type: &'a str,
    #[serde(rename = "serviceEndpoint")]
    service_endpoint: &'a str,
}

impl<'a> From<&'a DidDocument> for SerializableDoc<'a> {
    fn from(doc: &'a DidDocument) -> Self {
        Self {
            id: &doc.id,
            also_known_as: &doc.also_known_as,
            service: doc
                .service
                .iter()
                .map(|s| SerializableService {
                    id: &s.id,
                    service_type: &s.service_type,
                    service_endpoint: &s.service_endpoint,
                })
                .collect(),
        }
    }
}

fn build_dns_resolver(config: &IdentityConfig) -> anyhow::Result<TokioAsyncResolver> {
    let mut opts = ResolverOpts::default();
    opts.timeout = Duration::from_secs(3);
    opts.attempts = 2;
    // The gateway caches resolutions itself, with TTLs it controls.
    opts.cache_size = 0;

    if config.dns_nameservers.is_empty() {
        match hickory_resolver::system_conf::read_system_conf() {
            Ok((system_config, mut system_opts)) => {
                system_opts.timeout = opts.timeout;
                system_opts.attempts = opts.attempts;
                system_opts.cache_size = 0;
                return Ok(TokioAsyncResolver::tokio(system_config, system_opts));
            }
            Err(e) => {
                tracing::warn!(error = %e, "no system DNS config; falling back to Cloudflare");
                return Ok(TokioAsyncResolver::tokio(
                    ResolverConfig::cloudflare(),
                    opts,
                ));
            }
        }
    }

    let ips: Vec<_> = config.dns_nameservers.iter().map(|a| a.ip()).collect();
    let port = config.dns_nameservers[0].port();
    let group = NameServerConfigGroup::from_ips_clear(&ips, port, true);
    Ok(TokioAsyncResolver::tokio(
        ResolverConfig::from_parts(None, Vec::new(), group),
        opts,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> DidDocument {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn reads_the_pds_endpoint_by_service_id() {
        let d = doc(r##"{
              "id": "did:plc:abc",
              "alsoKnownAs": ["at://alice.rocksky.social"],
              "service": [
                {"id": "#other", "type": "Something", "serviceEndpoint": "https://nope.example"},
                {"id": "#atproto_pds", "type": "AtprotoPersonalDataServer",
                 "serviceEndpoint": "https://radxa.rocksky.social"}
              ]
            }"##);

        assert_eq!(d.pds_endpoint(), Some("https://radxa.rocksky.social"));
        assert_eq!(d.pds_host().as_deref(), Some("radxa.rocksky.social"));
        assert_eq!(d.claimed_handle(), Some("alice.rocksky.social"));
    }

    #[test]
    fn falls_back_to_the_service_type() {
        let d = doc(r##"{"id":"did:plc:abc","service":[
                 {"id":"#pds","type":"AtprotoPersonalDataServer",
                  "serviceEndpoint":"https://orangepi-zero-3w.rocksky.social"}]}"##);
        assert_eq!(
            d.pds_host().as_deref(),
            Some("orangepi-zero-3w.rocksky.social")
        );
    }

    #[test]
    fn tolerates_a_document_with_no_pds() {
        let d = doc(r##"{"id":"did:plc:abc","service":[]}"##);
        assert!(d.pds_endpoint().is_none());
        assert!(d.pds_host().is_none());
        assert!(d.claimed_handle().is_none());
    }

    #[test]
    fn ignores_an_empty_service_endpoint() {
        let d = doc(r##"{"id":"did:plc:abc","service":[
                 {"id":"#atproto_pds","type":"AtprotoPersonalDataServer","serviceEndpoint":""}]}"##);
        assert!(d.pds_endpoint().is_none());
    }

    #[test]
    fn lowercases_the_pds_host() {
        let d = doc(r##"{"id":"did:plc:abc","service":[
                 {"id":"#atproto_pds","type":"AtprotoPersonalDataServer",
                  "serviceEndpoint":"https://RADXA.Rocksky.Social"}]}"##);
        assert_eq!(d.pds_host().as_deref(), Some("radxa.rocksky.social"));
    }

    #[test]
    fn the_l2_mirror_round_trips() {
        let original = doc(
            r##"{"id":"did:plc:abc","alsoKnownAs":["at://alice.rocksky.social"],
                "service":[{"id":"#atproto_pds","type":"AtprotoPersonalDataServer",
                            "serviceEndpoint":"https://radxa.rocksky.social"}]}"##,
        );
        let json = serde_json::to_string(&SerializableDoc::from(&original)).unwrap();
        let back = doc(&json);

        assert_eq!(back.id, original.id);
        assert_eq!(back.pds_host(), original.pds_host());
        assert_eq!(back.claimed_handle(), original.claimed_handle());
    }
}
