//! Typed client errors shared by the CLI, MCP server, and Rig Desktop.

use thiserror::Error;

pub type ClientResult<T> = std::result::Result<T, ClientError>;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{0}")]
    Auth(String),
    #[error("{message}")]
    SecondFactorRequired {
        message: String,
        challenge_token: Option<String>,
    },
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    RateLimited(String),
    #[error("{0}")]
    Network(String),
    #[error("TLS certificate fingerprint mismatch (expected {expected}, server presented {actual})")]
    TlsPinMismatch { expected: String, actual: String },
    #[error("{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ClientError {
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Auth(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    pub fn rate_limited(msg: impl Into<String>) -> Self {
        Self::RateLimited(msg.into())
    }

    pub fn network(msg: impl Into<String>) -> Self {
        Self::Network(msg.into())
    }

    pub fn message(msg: impl Into<String>) -> Self {
        Self::Other(anyhow::anyhow!(msg.into()))
    }
}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        Self::Other(e.into())
    }
}

impl From<serde_json::Error> for ClientError {
    fn from(e: serde_json::Error) -> Self {
        Self::Other(e.into())
    }
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() || e.is_timeout() {
            Self::Network(e.without_url().to_string())
        } else {
            Self::Other(anyhow::anyhow!(e.without_url()))
        }
    }
}
