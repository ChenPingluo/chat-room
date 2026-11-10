use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("未登录")]
    Unauthorized,
    #[error("用户名或密码错误")]
    LoginFailed,
    #[error("{0}")]
    Forbidden(String),
    #[error("找不到")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    TooMany(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn bad(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn too_many(message: impl Into<String>) -> Self {
        Self::TooMany(message.into())
    }

    pub fn internal(err: impl Into<anyhow::Error>) -> Self {
        Self::Internal(err.into())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),
            Self::Unauthorized | Self::LoginFailed => (StatusCode::UNAUTHORIZED, self.to_string()),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message.clone()),
            Self::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            Self::Conflict(message) => (StatusCode::CONFLICT, message.clone()),
            Self::TooMany(message) => (StatusCode::TOO_MANY_REQUESTS, message.clone()),
            Self::Internal(err) => {
                tracing::error!(error = %err, "内部错误");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "服务器内部错误".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
