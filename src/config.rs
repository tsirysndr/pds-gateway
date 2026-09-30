//! TOML file overlaid with `GATEWAY_*` environment variables; env always wins.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::ConfigError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    #[default]
    LeastAccounts,
    RoundRobin,
    Weighted,
    Pinned,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FirehoseMode {
    /// Merge every node's stream under gateway-owned sequence numbers.
    #[default]
    Multiplex,
    /// Relay one node's stream, chosen with `?node=` or the default node.
    Passthrough,
    Off,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeConfig {
    pub name: String,
    pub url: Url,
    #[serde(default)]
    pub public_host: Option<String>,
    #[serde(default)]
    pub did: Option<String>,
    #[serde(default = "one")]
    pub weight: u32,
    #[serde(default = "yes")]
    pub accepts_signups: bool,
    #[serde(default)]
    pub max_accounts: Option<u64>,
}

fn one() -> u32 {
    1
}
fn yes() -> bool {
    true
}

impl NodeConfig {
    pub fn effective_public_host(&self) -> String {
        self.public_host
            .clone()
            .or_else(|| self.url.host_str().map(str::to_owned))
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub public_url: Url,
    pub did: Option<String>,
    pub trust_forwarded_headers: bool,
    pub shutdown_grace: DurationSetting,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:2583".parse().expect("valid default bind"),
            public_url: Url::parse("http://localhost:2583").expect("valid default url"),
            did: None,
            trust_forwarded_headers: true,
            shutdown_grace: DurationSetting::secs(20),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GatewayConfig {
    /// Domains whose handle namespace the gateway owns and protects.
    pub handle_domains: Vec<String>,
    pub reserved_handles: BTreeSet<String>,
    pub placement: Placement,
    /// Receives requests that carry no routable subject.
    pub default_node: Option<String>,
    pub reservation_ttl: DurationSetting,
    pub allow_signups: bool,
    /// Try `createSession` on every node when the identifier is an email, which
    /// the gateway cannot resolve to a DID on its own.
    pub broadcast_login: bool,
    pub honor_proxy_header: bool,
}

/// Newtype so `Duration` fields keep humantime spelling without repeating the
/// `#[serde(with)]` attribute on every one of them.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(transparent)]
pub struct DurationSetting(#[serde(with = "humantime_serde")] pub Duration);

impl std::ops::Deref for DurationSetting {
    type Target = Duration;
    fn deref(&self) -> &Duration {
        &self.0
    }
}

impl From<Duration> for DurationSetting {
    fn from(d: Duration) -> Self {
        Self(d)
    }
}

impl DurationSetting {
    const fn secs(s: u64) -> Self {
        Self(Duration::from_secs(s))
    }
    const fn millis(ms: u64) -> Self {
        Self(Duration::from_millis(ms))
    }
    pub fn get(self) -> Duration {
        self.0
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            handle_domains: Vec::new(),
            reserved_handles: [
                "admin",
                "administrator",
                "api",
                "app",
                "bsky",
                "dev",
                "did",
                "gateway",
                "help",
                "mod",
                "moderation",
                "official",
                "owner",
                "pds",
                "root",
                "security",
                "staff",
                "support",
                "sys",
                "system",
                "www",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
            placement: Placement::default(),
            default_node: None,
            reservation_ttl: DurationSetting::secs(600),
            allow_signups: true,
            broadcast_login: true,
            honor_proxy_header: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct IdentityConfig {
    pub plc_directory_url: Url,
    pub handle_cache_ttl: DurationSetting,
    pub handle_negative_cache_ttl: DurationSetting,
    pub did_cache_ttl: DurationSetting,
    pub route_cache_ttl: DurationSetting,
    pub cache_capacity: u64,
    /// Empty uses the system resolver.
    pub dns_nameservers: Vec<SocketAddr>,
    pub dns_resolution: bool,
    pub well_known_resolution: bool,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            plc_directory_url: Url::parse("https://plc.directory").expect("valid plc url"),
            handle_cache_ttl: DurationSetting::secs(300),
            handle_negative_cache_ttl: DurationSetting::secs(30),
            did_cache_ttl: DurationSetting::secs(600),
            route_cache_ttl: DurationSetting::secs(300),
            cache_capacity: 100_000,
            dns_nameservers: Vec::new(),
            dns_resolution: true,
            well_known_resolution: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UpstreamConfig {
    pub connect_timeout: DurationSetting,
    pub request_timeout: DurationSetting,
    /// Blobs and CAR transfers, which are slow.
    pub transfer_timeout: DurationSetting,
    pub pool_idle_timeout: DurationSetting,
    pub pool_max_idle_per_host: usize,
    /// Largest JSON body buffered to find a routing subject. Streaming
    /// endpoints are never buffered.
    pub max_buffered_body_bytes: usize,
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            connect_timeout: DurationSetting::secs(5),
            request_timeout: DurationSetting::secs(60),
            transfer_timeout: DurationSetting::secs(600),
            pool_idle_timeout: DurationSetting::secs(90),
            pool_max_idle_per_host: 32,
            max_buffered_body_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HealthConfig {
    pub interval: DurationSetting,
    pub timeout: DurationSetting,
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub probe_path: String,
    /// Keep routing to a down node (it may still serve reads). Placement avoids
    /// down nodes either way.
    pub route_to_unhealthy: bool,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            interval: DurationSetting::secs(15),
            timeout: DurationSetting::secs(3),
            failure_threshold: 3,
            success_threshold: 2,
            probe_path: "/xrpc/_health".to_owned(),
            route_to_unhealthy: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FirehoseConfig {
    pub mode: FirehoseMode,
    /// Frames kept in memory for cursor replay.
    pub replay_buffer: usize,
    pub reconnect_min_backoff: DurationSetting,
    pub reconnect_max_backoff: DurationSetting,
    /// A subscriber that falls further behind is disconnected.
    pub subscriber_queue: usize,
}

impl Default for FirehoseConfig {
    fn default() -> Self {
        Self {
            mode: FirehoseMode::default(),
            replay_buffer: 8192,
            reconnect_min_backoff: DurationSetting::millis(500),
            reconnect_max_backoff: DurationSetting::secs(30),
            subscriber_queue: 1024,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct StoreConfig {
    pub path: PathBuf,
    pub max_connections: u32,
    pub sweep_interval: DurationSetting,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("data/gateway.sqlite3"),
            max_connections: 8,
            sweep_interval: DurationSetting::secs(60),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct RedisConfig {
    pub url: Option<String>,
    pub key_prefix: String,
    /// Fall back to local-only behaviour when Redis is unreachable instead of
    /// failing requests.
    pub fail_open: bool,
    pub connect_timeout: DurationSetting,
    pub response_timeout: DurationSetting,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: None,
            key_prefix: "pdsgw".to_owned(),
            fail_open: true,
            connect_timeout: DurationSetting::secs(3),
            response_timeout: DurationSetting::secs(2),
        }
    }
}

impl RedisConfig {
    pub fn enabled(&self) -> bool {
        self.url.is_some()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct DelegateConfig {
    pub enabled: bool,
    /// Ask the nodes when the registry has no answer.
    pub fan_out: bool,
    /// Must stay under the caller's budget: atoll gives a delegate 2s to
    /// connect and 3s to answer, on the TLS handshake path.
    pub ask_timeout: DurationSetting,
    pub cache_ttl: DurationSetting,
    /// Answer `/tls-check` for names in the handle namespace, so on-demand TLS
    /// can issue certificates for them.
    pub tls_check: bool,
}

impl Default for DelegateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fan_out: true,
            ask_timeout: DurationSetting::millis(1500),
            cache_ttl: DurationSetting::secs(300),
            tls_check: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AdminConfig {
    /// Unset disables `/admin/*`.
    pub token: Option<String>,
    pub metrics: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub gateway: GatewayConfig,
    pub identity: IdentityConfig,
    pub upstream: UpstreamConfig,
    pub health: HealthConfig,
    pub firehose: FirehoseConfig,
    pub store: StoreConfig,
    pub redis: RedisConfig,
    pub delegate: DelegateConfig,
    pub admin: AdminConfig,
    #[serde(rename = "nodes")]
    pub nodes: Vec<NodeConfig>,
}

fn env_var(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn parse_env<T>(key: &str, slot: &mut T) -> Result<(), ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    if let Some(raw) = env_var(key) {
        *slot = raw
            .parse()
            .map_err(|e| ConfigError::Env(key.to_owned(), format!("{e}")))?;
    }
    Ok(())
}

fn parse_env_opt<T>(key: &str, slot: &mut Option<T>) -> Result<(), ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    if let Some(raw) = env_var(key) {
        *slot = Some(
            raw.parse()
                .map_err(|e| ConfigError::Env(key.to_owned(), format!("{e}")))?,
        );
    }
    Ok(())
}

fn parse_env_duration(key: &str, slot: &mut DurationSetting) -> Result<(), ConfigError> {
    if let Some(raw) = env_var(key) {
        *slot =
            DurationSetting(parse_duration(&raw).map_err(|e| ConfigError::Env(key.to_owned(), e))?);
    }
    Ok(())
}

fn parse_env_bool(key: &str, slot: &mut bool) -> Result<(), ConfigError> {
    if let Some(raw) = env_var(key) {
        *slot = parse_bool(&raw).ok_or_else(|| {
            ConfigError::Env(key.to_owned(), format!("expected a boolean, got `{raw}`"))
        })?;
    }
    Ok(())
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

fn parse_env_list(key: &str) -> Option<Vec<String>> {
    env_var(key).map(|raw| {
        raw.split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// Humantime spellings (`5m`, `1h30m`, `500ms`) plus a bare integer as seconds.
fn parse_duration(raw: &str) -> Result<Duration, String> {
    if let Ok(secs) = raw.parse::<u64>() {
        return Ok(Duration::from_secs(secs));
    }
    humantime_serde::re::humantime::parse_duration(raw).map_err(|e| format!("{e}"))
}

fn parse_nodes_env(raw: &str) -> Result<Vec<NodeConfig>, ConfigError> {
    let bad = |msg: String| ConfigError::Env("GATEWAY_NODES".to_owned(), msg);
    let mut nodes = Vec::new();

    for spec in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let fields: Vec<&str> = spec.split('|').map(str::trim).collect();
        if fields.len() < 2 {
            return Err(bad(format!(
                "`{spec}` needs at least `name|url` (got {} field(s))",
                fields.len()
            )));
        }
        if fields[0].is_empty() {
            return Err(bad(format!("`{spec}` has an empty node name")));
        }
        let url = Url::parse(fields[1])
            .map_err(|e| bad(format!("`{spec}` has an invalid url `{}`: {e}", fields[1])))?;
        let field = |i: usize| fields.get(i).copied().filter(|s| !s.is_empty());

        nodes.push(NodeConfig {
            name: fields[0].to_owned(),
            url,
            public_host: field(2).map(str::to_owned),
            did: field(5).map(str::to_owned),
            weight: match field(3) {
                Some(w) => w
                    .parse()
                    .map_err(|e| bad(format!("`{spec}` has an invalid weight `{w}`: {e}")))?,
                None => 1,
            },
            accepts_signups: field(4).and_then(parse_bool).unwrap_or(true),
            max_accounts: None,
        });
    }

    Ok(nodes)
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let explicit = path.map(Path::to_path_buf).or_else(|| {
            env_var("GATEWAY_CONFIG")
                .map(PathBuf::from)
                .filter(|p| p.exists())
        });

        let mut config = match explicit {
            Some(path) => {
                let raw = std::fs::read_to_string(&path)
                    .map_err(|e| ConfigError::Read(path.display().to_string(), e))?;
                let parsed = toml::from_str(&raw)
                    .map_err(|e| ConfigError::Toml(path.display().to_string(), e))?;
                tracing::info!(path = %path.display(), "loaded configuration file");
                parsed
            }
            None => {
                tracing::info!("no configuration file; using defaults and environment");
                Self::default()
            }
        };

        config.apply_env()?;
        config.normalize();
        config.validate()?;
        Ok(config)
    }

    fn apply_env(&mut self) -> Result<(), ConfigError> {
        parse_env("GATEWAY_BIND", &mut self.server.bind)?;
        parse_env("GATEWAY_PUBLIC_URL", &mut self.server.public_url)?;
        parse_env_opt("GATEWAY_DID", &mut self.server.did)?;
        parse_env_bool(
            "GATEWAY_TRUST_FORWARDED_HEADERS",
            &mut self.server.trust_forwarded_headers,
        )?;
        parse_env_duration("GATEWAY_SHUTDOWN_GRACE", &mut self.server.shutdown_grace)?;

        if let Some(domains) = parse_env_list("GATEWAY_HANDLE_DOMAINS") {
            self.gateway.handle_domains = domains;
        }
        if let Some(reserved) = parse_env_list("GATEWAY_RESERVED_HANDLES") {
            self.gateway.reserved_handles = reserved.into_iter().collect();
        }
        if let Some(raw) = env_var("GATEWAY_PLACEMENT") {
            self.gateway.placement = match raw.to_ascii_lowercase().replace('_', "-").as_str() {
                "least-accounts" | "least" => Placement::LeastAccounts,
                "round-robin" | "rr" => Placement::RoundRobin,
                "weighted" => Placement::Weighted,
                "pinned" | "default" => Placement::Pinned,
                other => {
                    return Err(ConfigError::Env(
                        "GATEWAY_PLACEMENT".to_owned(),
                        format!(
                            "unknown strategy `{other}`; expected least-accounts, \
                             round-robin, weighted or pinned"
                        ),
                    ));
                }
            };
        }
        parse_env_opt("GATEWAY_DEFAULT_NODE", &mut self.gateway.default_node)?;
        parse_env_duration("GATEWAY_RESERVATION_TTL", &mut self.gateway.reservation_ttl)?;
        parse_env_bool("GATEWAY_ALLOW_SIGNUPS", &mut self.gateway.allow_signups)?;
        parse_env_bool("GATEWAY_BROADCAST_LOGIN", &mut self.gateway.broadcast_login)?;
        parse_env_bool(
            "GATEWAY_HONOR_PROXY_HEADER",
            &mut self.gateway.honor_proxy_header,
        )?;

        parse_env(
            "GATEWAY_PLC_DIRECTORY_URL",
            &mut self.identity.plc_directory_url,
        )?;
        parse_env_duration(
            "GATEWAY_HANDLE_CACHE_TTL",
            &mut self.identity.handle_cache_ttl,
        )?;
        parse_env_duration(
            "GATEWAY_HANDLE_NEGATIVE_CACHE_TTL",
            &mut self.identity.handle_negative_cache_ttl,
        )?;
        parse_env_duration("GATEWAY_DID_CACHE_TTL", &mut self.identity.did_cache_ttl)?;
        parse_env_duration(
            "GATEWAY_ROUTE_CACHE_TTL",
            &mut self.identity.route_cache_ttl,
        )?;
        parse_env("GATEWAY_CACHE_CAPACITY", &mut self.identity.cache_capacity)?;
        parse_env_bool("GATEWAY_DNS_RESOLUTION", &mut self.identity.dns_resolution)?;
        parse_env_bool(
            "GATEWAY_WELL_KNOWN_RESOLUTION",
            &mut self.identity.well_known_resolution,
        )?;
        if let Some(servers) = parse_env_list("GATEWAY_DNS_NAMESERVERS") {
            self.identity.dns_nameservers = servers
                .iter()
                .map(|s| {
                    s.parse::<SocketAddr>()
                        .or_else(|_| format!("{s}:53").parse::<SocketAddr>())
                        .map_err(|e| {
                            ConfigError::Env(
                                "GATEWAY_DNS_NAMESERVERS".to_owned(),
                                format!("`{s}` is not an address: {e}"),
                            )
                        })
                })
                .collect::<Result<_, _>>()?;
        }

        parse_env_duration(
            "GATEWAY_UPSTREAM_CONNECT_TIMEOUT",
            &mut self.upstream.connect_timeout,
        )?;
        parse_env_duration(
            "GATEWAY_UPSTREAM_REQUEST_TIMEOUT",
            &mut self.upstream.request_timeout,
        )?;
        parse_env_duration(
            "GATEWAY_UPSTREAM_TRANSFER_TIMEOUT",
            &mut self.upstream.transfer_timeout,
        )?;
        parse_env_duration(
            "GATEWAY_UPSTREAM_POOL_IDLE_TIMEOUT",
            &mut self.upstream.pool_idle_timeout,
        )?;
        parse_env(
            "GATEWAY_UPSTREAM_POOL_MAX_IDLE_PER_HOST",
            &mut self.upstream.pool_max_idle_per_host,
        )?;
        parse_env(
            "GATEWAY_MAX_BUFFERED_BODY_BYTES",
            &mut self.upstream.max_buffered_body_bytes,
        )?;

        parse_env_duration("GATEWAY_HEALTH_INTERVAL", &mut self.health.interval)?;
        parse_env_duration("GATEWAY_HEALTH_TIMEOUT", &mut self.health.timeout)?;
        parse_env(
            "GATEWAY_HEALTH_FAILURE_THRESHOLD",
            &mut self.health.failure_threshold,
        )?;
        parse_env(
            "GATEWAY_HEALTH_SUCCESS_THRESHOLD",
            &mut self.health.success_threshold,
        )?;
        parse_env("GATEWAY_HEALTH_PROBE_PATH", &mut self.health.probe_path)?;
        parse_env_bool(
            "GATEWAY_ROUTE_TO_UNHEALTHY",
            &mut self.health.route_to_unhealthy,
        )?;

        if let Some(raw) = env_var("GATEWAY_FIREHOSE_MODE") {
            self.firehose.mode = match raw.to_ascii_lowercase().as_str() {
                "multiplex" | "merge" => FirehoseMode::Multiplex,
                "passthrough" | "proxy" => FirehoseMode::Passthrough,
                "off" | "disabled" | "none" => FirehoseMode::Off,
                other => {
                    return Err(ConfigError::Env(
                        "GATEWAY_FIREHOSE_MODE".to_owned(),
                        format!("unknown mode `{other}`; expected multiplex, passthrough or off"),
                    ));
                }
            };
        }
        parse_env(
            "GATEWAY_FIREHOSE_REPLAY_BUFFER",
            &mut self.firehose.replay_buffer,
        )?;
        parse_env(
            "GATEWAY_FIREHOSE_SUBSCRIBER_QUEUE",
            &mut self.firehose.subscriber_queue,
        )?;
        parse_env_duration(
            "GATEWAY_FIREHOSE_RECONNECT_MIN_BACKOFF",
            &mut self.firehose.reconnect_min_backoff,
        )?;
        parse_env_duration(
            "GATEWAY_FIREHOSE_RECONNECT_MAX_BACKOFF",
            &mut self.firehose.reconnect_max_backoff,
        )?;

        parse_env("GATEWAY_STORE_PATH", &mut self.store.path)?;
        parse_env(
            "GATEWAY_STORE_MAX_CONNECTIONS",
            &mut self.store.max_connections,
        )?;
        parse_env_duration(
            "GATEWAY_STORE_SWEEP_INTERVAL",
            &mut self.store.sweep_interval,
        )?;

        parse_env_opt("GATEWAY_REDIS_URL", &mut self.redis.url)?;
        parse_env("GATEWAY_REDIS_KEY_PREFIX", &mut self.redis.key_prefix)?;
        parse_env_bool("GATEWAY_REDIS_FAIL_OPEN", &mut self.redis.fail_open)?;
        parse_env_duration(
            "GATEWAY_REDIS_CONNECT_TIMEOUT",
            &mut self.redis.connect_timeout,
        )?;
        parse_env_duration(
            "GATEWAY_REDIS_RESPONSE_TIMEOUT",
            &mut self.redis.response_timeout,
        )?;

        parse_env_bool("GATEWAY_DELEGATE_ENABLED", &mut self.delegate.enabled)?;
        parse_env_bool("GATEWAY_DELEGATE_FAN_OUT", &mut self.delegate.fan_out)?;
        parse_env_duration(
            "GATEWAY_DELEGATE_ASK_TIMEOUT",
            &mut self.delegate.ask_timeout,
        )?;
        parse_env_duration("GATEWAY_DELEGATE_CACHE_TTL", &mut self.delegate.cache_ttl)?;
        parse_env_bool("GATEWAY_DELEGATE_TLS_CHECK", &mut self.delegate.tls_check)?;

        parse_env_opt("GATEWAY_ADMIN_TOKEN", &mut self.admin.token)?;
        parse_env_bool("GATEWAY_METRICS", &mut self.admin.metrics)?;

        if let Some(raw) = env_var("GATEWAY_NODES") {
            self.nodes = parse_nodes_env(&raw)?;
        }

        Ok(())
    }

    fn normalize(&mut self) {
        for domain in &mut self.gateway.handle_domains {
            *domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        }
        self.gateway.handle_domains.sort();
        self.gateway.handle_domains.dedup();

        self.gateway.reserved_handles = self
            .gateway
            .reserved_handles
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();

        for node in &mut self.nodes {
            node.name = node.name.trim().to_ascii_lowercase();
            if let Some(host) = &mut node.public_host {
                *host = host.trim().to_ascii_lowercase();
            }
            // A trailing slash would double up when joined with request paths.
            if node.url.path() == "/" {
                node.url.set_path("");
            }
        }

        if !self.health.probe_path.starts_with('/') {
            self.health.probe_path.insert(0, '/');
        }

        if self.gateway.default_node.is_none() && self.nodes.len() == 1 {
            self.gateway.default_node = Some(self.nodes[0].name.clone());
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.nodes.is_empty() {
            return Err(ConfigError::Invalid(
                "no upstream PDS nodes configured; add [[nodes]] entries or set GATEWAY_NODES"
                    .to_owned(),
            ));
        }

        let mut names = BTreeSet::new();
        let mut hosts = BTreeSet::new();
        for node in &self.nodes {
            if !names.insert(node.name.clone()) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate node name `{}`",
                    node.name
                )));
            }
            if node.url.host_str().is_none() {
                return Err(ConfigError::Invalid(format!(
                    "node `{}` has a url without a host: {}",
                    node.name, node.url
                )));
            }
            if !matches!(node.url.scheme(), "http" | "https") {
                return Err(ConfigError::Invalid(format!(
                    "node `{}` must use http or https, got `{}`",
                    node.name,
                    node.url.scheme()
                )));
            }
            let host = node.effective_public_host();
            if !hosts.insert(host.clone()) {
                return Err(ConfigError::Invalid(format!(
                    "node `{}` shares public_host `{host}` with another node; routing by \
                     DID document would be ambiguous",
                    node.name
                )));
            }
            if node.weight == 0 && self.gateway.placement == Placement::Weighted {
                return Err(ConfigError::Invalid(format!(
                    "node `{}` has weight 0 under weighted placement",
                    node.name
                )));
            }
        }

        match &self.gateway.default_node {
            Some(default) if !names.contains(default) => {
                return Err(ConfigError::Invalid(format!(
                    "default_node `{default}` is not one of the configured nodes: {}",
                    names.into_iter().collect::<Vec<_>>().join(", ")
                )));
            }
            None => {
                return Err(ConfigError::Invalid(
                    "default_node is required with more than one node; it receives \
                     requests that carry no routable subject"
                        .to_owned(),
                ));
            }
            _ => {}
        }

        if self.gateway.handle_domains.is_empty() {
            return Err(ConfigError::Invalid(
                "handle_domains is empty; the gateway would own no handle namespace and \
                 could not prevent collisions"
                    .to_owned(),
            ));
        }

        if self.gateway.allow_signups {
            if self.gateway.placement == Placement::Pinned {
                let default = self.gateway.default_node.as_deref().unwrap_or_default();
                if !self
                    .nodes
                    .iter()
                    .any(|n| n.name == default && n.accepts_signups)
                {
                    return Err(ConfigError::Invalid(format!(
                        "pinned placement targets `{default}`, which does not accept signups"
                    )));
                }
            } else if !self.nodes.iter().any(|n| n.accepts_signups) {
                return Err(ConfigError::Invalid(
                    "signups are allowed but no node has accepts_signups = true".to_owned(),
                ));
            }
        }

        if self.health.failure_threshold == 0 || self.health.success_threshold == 0 {
            return Err(ConfigError::Invalid(
                "health failure_threshold and success_threshold must be at least 1".to_owned(),
            ));
        }

        if self.firehose.mode == FirehoseMode::Multiplex && self.firehose.replay_buffer == 0 {
            return Err(ConfigError::Invalid(
                "firehose replay_buffer must be at least 1 in multiplex mode".to_owned(),
            ));
        }

        if self.admin.token.as_deref().is_some_and(str::is_empty) {
            return Err(ConfigError::Invalid(
                "admin.token is set but empty; unset it to disable the admin API".to_owned(),
            ));
        }

        if let Some(url) = &self.redis.url
            && !url.starts_with("redis://")
            && !url.starts_with("rediss://")
            && !url.starts_with("unix:")
        {
            return Err(ConfigError::Invalid(format!(
                "redis url `{url}` must start with redis://, rediss:// or unix:"
            )));
        }

        Ok(())
    }

    pub fn node(&self, name: &str) -> Option<&NodeConfig> {
        self.nodes.iter().find(|n| n.name == name)
    }

    /// True when `handle` sits under a domain the gateway owns.
    pub fn owns_handle(&self, handle: &str) -> bool {
        let handle = handle.to_ascii_lowercase();
        self.gateway
            .handle_domains
            .iter()
            .any(|d| handle.len() > d.len() + 1 && handle.ends_with(&format!(".{d}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, url: &str, host: &str) -> NodeConfig {
        NodeConfig {
            name: name.into(),
            url: Url::parse(url).unwrap(),
            public_host: Some(host.into()),
            did: None,
            weight: 1,
            accepts_signups: true,
            max_accounts: None,
        }
    }

    fn base() -> Config {
        Config {
            gateway: GatewayConfig {
                handle_domains: vec!["rocksky.social".into()],
                default_node: Some("primary".into()),
                ..GatewayConfig::default()
            },
            nodes: vec![
                node("primary", "http://127.0.0.1:2584", "rocksky.social"),
                node("radxa", "http://radxa.lan:2583", "radxa.rocksky.social"),
            ],
            ..Config::default()
        }
    }

    #[test]
    fn accepts_a_well_formed_fleet() {
        let mut config = base();
        config.normalize();
        config.validate().unwrap();
    }

    #[test]
    fn rejects_duplicate_public_hosts() {
        let mut config = base();
        config.nodes[1].public_host = Some("rocksky.social".into());
        config.normalize();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("shares public_host"), "{err}");
    }

    #[test]
    fn rejects_unknown_default_node() {
        let mut config = base();
        config.gateway.default_node = Some("nope".into());
        config.normalize();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("not one of the configured nodes"), "{err}");
    }

    #[test]
    fn single_node_needs_no_explicit_default() {
        let mut config = base();
        config.nodes.truncate(1);
        config.gateway.default_node = None;
        config.normalize();
        config.validate().unwrap();
        assert_eq!(config.gateway.default_node.as_deref(), Some("primary"));
    }

    #[test]
    fn owns_only_subdomains_of_its_handle_domains() {
        let config = base();
        assert!(config.owns_handle("alice.rocksky.social"));
        assert!(config.owns_handle("ALICE.ROCKSKY.SOCIAL"));
        // The bare domain is the gateway itself, not a user handle.
        assert!(!config.owns_handle("rocksky.social"));
        assert!(!config.owns_handle("alice.bsky.social"));
        assert!(!config.owns_handle("alice.notrocksky.social"));
    }

    #[test]
    fn parses_the_compact_node_spec() {
        let nodes = parse_nodes_env(
            "primary|http://127.0.0.1:2584|rocksky.social, \
             radxa|http://radxa.lan:2583|radxa.rocksky.social|3|false|did:web:radxa.rocksky.social",
        )
        .unwrap();

        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "primary");
        assert_eq!(nodes[0].weight, 1);
        assert!(nodes[0].accepts_signups);
        assert_eq!(nodes[1].weight, 3);
        assert!(!nodes[1].accepts_signups);
        assert_eq!(
            nodes[1].did.as_deref(),
            Some("did:web:radxa.rocksky.social")
        );
    }

    #[test]
    fn rejects_a_node_spec_without_a_url() {
        let err = parse_nodes_env("primary").unwrap_err().to_string();
        assert!(err.contains("name|url"), "{err}");
    }

    #[test]
    fn durations_accept_bare_seconds_and_humantime() {
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn a_full_toml_round_trips() {
        let toml_src = r#"
[server]
bind = "0.0.0.0:3000"
public_url = "https://rocksky.social"

[gateway]
handle_domains = ["rocksky.social"]
default_node = "primary"
placement = "round-robin"
reservation_ttl = "15m"

[redis]
url = "redis://127.0.0.1:6379"

[[nodes]]
name = "primary"
url = "http://127.0.0.1:2584"
public_host = "rocksky.social"

[[nodes]]
name = "radxa"
url = "http://radxa.lan:2583"
public_host = "radxa.rocksky.social"
weight = 2
"#;
        let mut config: Config = toml::from_str(toml_src).unwrap();
        config.normalize();
        config.validate().unwrap();

        assert_eq!(config.server.bind.port(), 3000);
        assert_eq!(config.gateway.placement, Placement::RoundRobin);
        assert_eq!(
            config.gateway.reservation_ttl.get(),
            Duration::from_secs(900)
        );
        assert!(config.redis.enabled());
        assert_eq!(config.nodes[1].weight, 2);
        // Untouched sections keep their defaults.
        assert_eq!(config.health.failure_threshold, 3);
    }

    #[test]
    fn rejects_a_bad_redis_url() {
        let mut config = base();
        config.redis.url = Some("localhost:6379".into());
        config.normalize();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("must start with redis://"), "{err}");
    }
}
