use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rig_api::{ErrorBody, ErrorDetail};

/// An error that is safe to return to a client.
///
/// Internal failures are logged with their cause and returned as an opaque
/// 500, so database or engine details never reach the response body.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Validation(String),
    #[error("not signed in")]
    Unauthorized,
    #[error("enter the code from your authenticator app")]
    SecondFactorRequired(Option<String>),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("too many attempts, try again later")]
    RateLimited,
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }

    fn parts(&self) -> (StatusCode, &'static str) {
        match self {
            Self::Validation(_) => (StatusCode::UNPROCESSABLE_ENTITY, "validation"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::SecondFactorRequired(_) => (StatusCode::UNAUTHORIZED, "second_factor_required"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e
            && db.is_unique_violation()
        {
            return Self::Conflict("that name is already in use".into());
        }
        Self::Internal(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = self.parts();
        let message = match &self {
            Self::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                "something went wrong on the server".to_string()
            }
            other => other.to_string(),
        };
        let challenge_token = match &self {
            Self::SecondFactorRequired(tok) => tok.clone(),
            _ => None,
        };
        let body = ErrorBody {
            error: ErrorDetail {
                code: code.into(),
                message,
                challenge_token,
            },
        };
        (status, Json(body)).into_response()
    }
}
