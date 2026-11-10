use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sqlx::FromRow;

use crate::error::AppError;
use crate::state::AppState;
use crate::util::{self, now_ms, session_cookie_name};

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub uid: String,
    pub username: String,
    pub is_admin: bool,
    pub session_id: i64,
}

#[derive(FromRow)]
struct SessionJoin {
    session_id: i64,
    expires_at: i64,
    last_seen: i64,
    id: i64,
    uid: String,
    username: String,
    is_admin: i64,
    disabled: i64,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let name = session_cookie_name(state.config.cookie_secure);
        let token = util::read_cookie(header, name).ok_or(AppError::Unauthorized)?;
        let hash = util::token_hash(token);
        let row = sqlx::query_as::<_, SessionJoin>(
            "SELECT s.id AS session_id, s.expires_at, s.last_seen, u.id, u.uid, u.username, u.is_admin, u.disabled
             FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?",
        )
        .bind(&hash)
        .fetch_optional(&state.db)
        .await?;
        let Some(row) = row else {
            return Err(AppError::Unauthorized);
        };
        let now = now_ms();
        if row.expires_at <= now || row.disabled != 0 {
            let _ = sqlx::query("DELETE FROM sessions WHERE id = ?")
                .bind(row.session_id)
                .execute(&state.db)
                .await;
            return Err(AppError::Unauthorized);
        }
        if now - row.last_seen > 5 * 60 * 1000 {
            let _ = sqlx::query("UPDATE sessions SET last_seen = ? WHERE id = ?")
                .bind(now)
                .bind(row.session_id)
                .execute(&state.db)
                .await;
        }
        Ok(AuthUser {
            id: row.id,
            uid: row.uid,
            username: row.username,
            is_admin: row.is_admin != 0,
            session_id: row.session_id,
        })
    }
}

pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|info| info.0.ip());
        Ok(ClientIp(util::client_ip(&parts.headers, peer)))
    }
}
