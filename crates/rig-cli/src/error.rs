//! Typed CLI errors mapped to stable process exit codes.
//!
//! Exit codes:
//! - `0`   success
//! - `1`   generic / server error
//! - `2`   invalid CLI usage (clap)
//! - `3`   authentication / not signed in / forbidden
//! - `4`   resource not found
//! - `5`   network / server unreachable / interrupted stream
//! - `10`  deployment failed or cancelled
//! - `130` detached via Ctrl-C while a deployment continues on the server

use thiserror::Error;

pub type CliResult<T> = std::result::Result<T, CliError>;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    DeployFailed(String),
    #[error("{0}")]
    Detached(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl CliError {
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Auth(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn network(msg: impl Into<String>) -> Self {
        Self::Network(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    pub fn deploy_failed(msg: impl Into<String>) -> Self {
        Self::DeployFailed(msg.into())
    }

    pub fn detached(msg: impl Into<String>) -> Self {
        Self::Detached(msg.into())
    }

    pub fn message(msg: impl Into<String>) -> Self {
        Self::Other(anyhow::anyhow!(msg.into()))
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Auth(_) => 3,
            Self::NotFound(_) => 4,
            Self::Network(_) => 5,
            Self::DeployFailed(_) => 10,
            Self::Detached(_) => 130,
            Self::Conflict(_) | Self::Other(_) => 1,
        }
    }

    /// True when the error message is already formatted for the user without a `rig: ` prefix.
    pub fn is_detached(&self) -> bool {
        matches!(self, Self::Detached(_))
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        Self::Other(e.into())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        Self::Other(e.into())
    }
}

impl From<toml::de::Error> for CliError {
    fn from(e: toml::de::Error) -> Self {
        Self::Other(e.into())
    }
}

impl From<toml::ser::Error> for CliError {
    fn from(e: toml::ser::Error) -> Self {
        Self::Other(e.into())
    }
}

impl From<reqwest::Error> for CliError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() || e.is_timeout() {
            Self::Network(e.without_url().to_string())
        } else {
            Self::Other(anyhow::anyhow!(e.without_url()))
        }
    }
}

impl From<rig_client::ClientError> for CliError {
    fn from(e: rig_client::ClientError) -> Self {
        match e {
            rig_client::ClientError::Auth(m) => Self::Auth(m),
            rig_client::ClientError::SecondFactorRequired { message, .. } => Self::Auth(message),
            rig_client::ClientError::NotFound(m) => Self::NotFound(m),
            rig_client::ClientError::Conflict(m) => Self::Conflict(m),
            rig_client::ClientError::Network(m) => Self::Network(m),
            other => Self::Other(anyhow::anyhow!(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct() {
        assert_eq!(CliError::message("boom").exit_code(), 1);
        assert_eq!(CliError::auth("no token").exit_code(), 3);
        assert_eq!(CliError::not_found("no app").exit_code(), 4);
        assert_eq!(CliError::network("unreachable").exit_code(), 5);
        assert_eq!(CliError::deploy_failed("failed").exit_code(), 10);
        assert_eq!(CliError::detached("detached").exit_code(), 130);
    }
}
