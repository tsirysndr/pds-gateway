//! Forwarding XRPC calls upstream.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use futures::TryStreamExt;

use crate::config::{Config, NodeConfig};
use crate::error::{GatewayError, Result};
use crate::routing::lexicon;

/// Headers that belong to one hop and must not be copied through.
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
];

pub struct UpstreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub node: String,
}

impl UpstreamResponse {
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// A request body that is either already in memory or still streaming.
pub enum RequestBody {
    Empty,
    Buffered(Bytes),
    Stream(Body),
}

pub struct Forwarder {
    config: Arc<Config>,
    http: reqwest::Client,
}

impl Forwarder {
    pub fn new(config: Arc<Config>, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    fn node(&self, name: &str) -> Result<&NodeConfig> {
        self.config
            .node(name)
            .ok_or_else(|| GatewayError::Internal(format!("unknown node `{name}`")))
    }

    fn timeout_for(&self, nsid: &str) -> Duration {
        if lexicon::is_streaming(nsid) {
            self.config.upstream.transfer_timeout.get()
        } else {
            self.config.upstream.request_timeout.get()
        }
    }

    /// Streams a call to `node` and streams the reply straight back, so blobs and
    /// CAR files never sit in gateway memory.
    #[allow(clippy::too_many_arguments)]
    pub async fn forward(
        &self,
        node_name: &str,
        method: Method,
        nsid: &str,
        query: &str,
        headers: &HeaderMap,
        body: RequestBody,
        client_ip: Option<std::net::IpAddr>,
    ) -> Result<Response> {
        let node = self.node(node_name)?;
        let url = build_url(node, nsid, query);

        let mut request = self
            .http
            .request(method.clone(), &url)
            .timeout(self.timeout_for(nsid));

        for (name, value) in forwardable(headers) {
            request = request.header(name, value);
        }
        request = request.header("host", node.effective_public_host());
        if let Some(ip) = client_ip {
            request = request.header("x-forwarded-for", ip.to_string());
        }
        request = request.header("x-forwarded-proto", self.config.server.public_url.scheme());
        if let Some(host) = self.config.server.public_url.host_str() {
            request = request.header("x-forwarded-host", host);
        }

        request = match body {
            RequestBody::Empty => request,
            RequestBody::Buffered(bytes) => request.body(bytes),
            RequestBody::Stream(body) => {
                let stream = body
                    .into_data_stream()
                    .map_err(|e| std::io::Error::other(e.to_string()));
                request.body(reqwest::Body::wrap_stream(stream))
            }
        };

        let response = request
            .send()
            .await
            .map_err(|e| self.upstream_error(node_name, e))?;

        crate::metrics::PROXIED.incr();
        let status = response.status();
        let mut builder = Response::builder().status(status);

        for (name, value) in response.headers() {
            if !HOP_BY_HOP.contains(&name.as_str()) {
                builder = builder.header(name, value);
            }
        }
        builder = builder.header("x-pdsgw-node", node_name);

        let stream = response
            .bytes_stream()
            .map_err(|e| std::io::Error::other(e.to_string()));

        builder
            .body(Body::from_stream(stream))
            .map_err(GatewayError::internal)
    }

    /// Forwards and buffers the reply. Used where the gateway must read the
    /// response, such as learning a DID after account creation.
    pub async fn forward_buffered(
        &self,
        node_name: &str,
        method: Method,
        nsid: &str,
        query: &str,
        headers: &HeaderMap,
        body: Option<Bytes>,
    ) -> Result<UpstreamResponse> {
        let node = self.node(node_name)?;
        let url = build_url(node, nsid, query);

        let mut request = self
            .http
            .request(method, &url)
            .timeout(self.timeout_for(nsid));

        for (name, value) in forwardable(headers) {
            request = request.header(name, value);
        }
        request = request.header("host", node.effective_public_host());

        if let Some(bytes) = body {
            request = request.body(bytes);
        }

        let response = request
            .send()
            .await
            .map_err(|e| self.upstream_error(node_name, e))?;
        crate::metrics::PROXIED.incr();

        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .bytes()
            .await
            .map_err(|e| self.upstream_error(node_name, e))?;

        Ok(UpstreamResponse {
            status,
            headers,
            body,
            node: node_name.to_owned(),
        })
    }

    /// Runs the same call against several nodes at once, for fan-out reads.
    pub async fn fanout(
        &self,
        nodes: &[String],
        method: Method,
        nsid: &str,
        query: &str,
        headers: &HeaderMap,
        body: Option<Bytes>,
    ) -> Vec<Result<UpstreamResponse>> {
        let calls = nodes.iter().map(|node| {
            self.forward_buffered(node, method.clone(), nsid, query, headers, body.clone())
        });
        futures::future::join_all(calls).await
    }

    fn upstream_error(&self, node: &str, error: reqwest::Error) -> GatewayError {
        crate::metrics::UPSTREAM_ERRORS.incr();
        if error.is_timeout() {
            GatewayError::UpstreamTimeout {
                node: node.to_owned(),
                elapsed: self.config.upstream.request_timeout.get(),
            }
        } else {
            GatewayError::Upstream {
                node: node.to_owned(),
                source: error,
            }
        }
    }
}

fn build_url(node: &NodeConfig, nsid: &str, query: &str) -> String {
    let base = node.url.as_str().trim_end_matches('/');
    if query.is_empty() {
        format!("{base}/xrpc/{nsid}")
    } else {
        format!("{base}/xrpc/{nsid}?{query}")
    }
}

fn forwardable(headers: &HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
    headers
        .iter()
        .filter(|(name, _)| !HOP_BY_HOP.contains(&name.as_str()))
        .filter(|(name, _)| !name.as_str().starts_with("x-forwarded-"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn node() -> NodeConfig {
        NodeConfig {
            name: "radxa".into(),
            url: Url::parse("https://radxa.rocksky.social").unwrap(),
            public_host: Some("radxa.rocksky.social".into()),
            did: None,
            weight: 1,
            accepts_signups: true,
            max_accounts: None,
        }
    }

    #[test]
    fn builds_xrpc_urls_with_and_without_a_query() {
        assert_eq!(
            build_url(&node(), "com.atproto.repo.getRecord", ""),
            "https://radxa.rocksky.social/xrpc/com.atproto.repo.getRecord"
        );
        assert_eq!(
            build_url(
                &node(),
                "com.atproto.repo.getRecord",
                "repo=did:plc:a&rkey=b"
            ),
            "https://radxa.rocksky.social/xrpc/com.atproto.repo.getRecord?repo=did:plc:a&rkey=b"
        );
    }

    #[test]
    fn a_trailing_slash_does_not_double_up() {
        let mut n = node();
        n.url = Url::parse("https://radxa.rocksky.social/").unwrap();
        assert_eq!(
            build_url(&n, "_health", ""),
            "https://radxa.rocksky.social/xrpc/_health"
        );
    }

    #[test]
    fn strips_hop_by_hop_and_client_forwarded_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer token".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("connection", "keep-alive".parse().unwrap());
        headers.insert("transfer-encoding", "chunked".parse().unwrap());
        headers.insert("host", "rocksky.social".parse().unwrap());
        // A client must not be able to forge its own origin.
        headers.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());

        let kept: Vec<_> = forwardable(&headers)
            .into_iter()
            .map(|(n, _)| n.as_str().to_owned())
            .collect();

        assert!(kept.contains(&"authorization".to_owned()));
        assert!(kept.contains(&"content-type".to_owned()));
        for dropped in ["connection", "transfer-encoding", "host", "x-forwarded-for"] {
            assert!(!kept.contains(&dropped.to_owned()), "{dropped} leaked");
        }
    }
}
