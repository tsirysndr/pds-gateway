//! Errors and their XRPC wire representation.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config file {0}: {1}")]
    Read(String, #[source] std::io::Error),
    #[error("could not parse config file {0}: {1}")]
    Toml(String, #[source] toml::de::Error),
    #[error("{0} is invalid: {1}")]
    Env(String, String),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("{0}")]
    InvalidRequest(String),

    #[error("authentication required")]
    AuthRequired,

    #[error("{0}")]
    Forbidden(String),

    #[error("could not resolve `{subject}`: {reason}")]
    UnresolvableSubject { subject: String, reason: String },

    #[error("handle `{0}` is already taken")]
    HandleTaken(String),

    #[error("{0}")]
    InvalidHandle(String),

    #[error("no upstream node can accept this request: {0}")]
    NoNodeAvailable(String),

    #[error("upstream node `{node}` failed: {source}")]
    Upstream {
        node: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("upstream node `{node}` timed out after {elapsed:?}")]
    UpstreamTimeout {
        node: String,
        elapsed: std::time::Duration,
    },

    #[error("request body is larger than the {limit} byte routing limit")]
    BodyTooLarge { limit: usize },

    #[error("storage failure: {0}")]
    Store(#[from] sqlx::Error),

    #[error("{0} is not supported by this gateway")]
    NotImplemented(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl GatewayError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::Internal(e.to_string())
    }

    /// The XRPC `error` name and HTTP status for this failure.
    fn wire(&self) -> (StatusCode, &'static str) {
        match self {
            Self::InvalidRequest(_) => (StatusCode::BAD_REQUEST, "InvalidRequest"),
            Self::AuthRequired => (StatusCode::UNAUTHORIZED, "AuthenticationRequired"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "Forbidden"),
            Self::UnresolvableSubject { .. } => (StatusCode::BAD_REQUEST, "InvalidRequest"),
            Self::HandleTaken(_) => (StatusCode::BAD_REQUEST, "HandleNotAvailable"),
            Self::InvalidHandle(_) => (StatusCode::BAD_REQUEST, "InvalidHandle"),
            Self::NoNodeAvailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "UpstreamUnavailable"),
            Self::Upstream { .. } => (StatusCode::BAD_GATEWAY, "UpstreamFailure"),
            Self::UpstreamTimeout { .. } => (StatusCode::GATEWAY_TIMEOUT, "UpstreamTimeout"),
            Self::BodyTooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, "RequestEntityTooLarge"),
            Self::NotImplemented(_) => (StatusCode::NOT_IMPLEMENTED, "MethodNotImplemented"),
            Self::Store(_) | Self::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "InternalServerError")
            }
        }
    }
}

#[derive(Serialize)]
pub struct XrpcError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl XrpcError {
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: Some(message.into()),
        }
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, name) = self.wire();

        // Internal failures get a generic message on the wire and the detail in
        // the log, so upstream internals are not echoed to clients.
        let message = match &self {
            Self::Store(_) | Self::Internal(_) => {
                tracing::error!(error = %self, "request failed");
                "the gateway could not complete this request".to_owned()
            }
            _ => {
                tracing::debug!(error = %self, xrpc_error = name, "request rejected");
                self.to_string()
            }
        };

        (status, Json(XrpcError::new(name, message))).into_response()
    }
}

pub type Result<T, E = GatewayError> = std::result::Result<T, E>;
