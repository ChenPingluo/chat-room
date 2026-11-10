use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum AppErr {
    BadRequest(&'static str),
    Unauthorized(&'static str),
    Forbidden(&'static str),
    NotFound,
    Conflict(&'static str),
    TooMany,
    TotpRequired,
    Internal,
}

impl AppErr {
    pub fn status(&self) -> StatusCode {
        match self {
            AppErr::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppErr::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppErr::Forbidden(_) => StatusCode::FORBIDDEN,
            AppErr::NotFound => StatusCode::NOT_FOUND,
            AppErr::Conflict(_) => StatusCode::CONFLICT,
            AppErr::TooMany => StatusCode::TOO_MANY_REQUESTS,
            AppErr::TotpRequired => StatusCode::UNAUTHORIZED,
            AppErr::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            AppErr::BadRequest(m)
            | AppErr::Unauthorized(m)
            | AppErr::Forbidden(m)
            | AppErr::Conflict(m) => m,
            AppErr::NotFound => "没有找到",
            AppErr::TooMany => "操作太频繁，请稍后再试",
            AppErr::TotpRequired => "需要两步验证码",
            AppErr::Internal => "服务器内部错误",
        }
    }
}

impl IntoResponse for AppErr {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message() });
        if matches!(self, AppErr::TotpRequired) {
            body["totp_required"] = json!(true);
        }
        (self.status(), Json(body)).into_response()
    }
}

impl From<rusqlite::Error> for AppErr {
    fn from(err: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(code, _) = &err {
            if code.code == rusqlite::ErrorCode::ConstraintViolation {
                return AppErr::Conflict("记录已存在");
            }
        }
        tracing::error!(error = %err, "数据库错误");
        AppErr::Internal
    }
}

#[derive(Debug)]
pub enum StartupError {
    BadKey,
    Message(&'static str),
    Io(std::io::Error),
    Db(rusqlite::Error),
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartupError::BadKey => write!(f, "主密钥无法解密现有数据，已拒绝启动"),
            StartupError::Message(m) => write!(f, "{m}"),
            StartupError::Io(e) => write!(f, "文件错误: {e}"),
            StartupError::Db(e) => write!(f, "数据库错误: {e}"),
        }
    }
}

impl std::error::Error for StartupError {}

impl From<std::io::Error> for StartupError {
    fn from(value: std::io::Error) -> Self {
        StartupError::Io(value)
    }
}

impl From<rusqlite::Error> for StartupError {
    fn from(value: rusqlite::Error) -> Self {
        StartupError::Db(value)
    }
}
