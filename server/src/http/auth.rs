use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::{AuthUser, ClientIp};
use crate::domain::auth as accounts;
use crate::dto::MeDto;
use crate::error::AppError;
use crate::http::JsonIn;
use crate::state::AppState;
use crate::util;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/invite/preview", post(preview_invite))
        .route("/me", get(me))
        .route("/me/password", post(change_password))
        .route("/sessions", get(sessions))
        .route("/sessions/logout-all", post(logout_all))
        .route("/sessions/{id}", axum::routing::delete(revoke_session))
        .route("/invites", get(list_invites).post(create_invite))
        .route("/invites/{id}/revoke", post(revoke_invite))
}

#[derive(Deserialize)]
struct RegisterBody {
    token: String,
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

#[derive(Deserialize)]
struct PasswordBody {
    current_password: String,
    new_password: String,
}

#[derive(Deserialize)]
struct InviteBody {
    #[serde(default = "default_days")]
    days: i64,
}

fn default_days() -> i64 {
    7
}

fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn with_cookie(
    state: &AppState,
    status: StatusCode,
    token: &str,
    body: MeDto,
) -> impl axum::response::IntoResponse {
    let cookie = util::session_cookie(
        state.config.cookie_secure,
        token,
        util::SESSION_TTL_MS / 1000,
    );
    (status, [(header::SET_COOKIE, cookie)], Json(body))
}

fn clear_cookie(state: &AppState) -> String {
    util::session_cookie(state.config.cookie_secure, "", 0)
}

async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    JsonIn(body): JsonIn<RegisterBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let (me, token) = accounts::register(
        &state,
        &body.token,
        &body.username,
        &body.password,
        &user_agent(&headers),
    )
    .await?;
    Ok(with_cookie(&state, StatusCode::CREATED, &token, me))
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ClientIp(ip): ClientIp,
    JsonIn(body): JsonIn<LoginBody>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let (me, token) = accounts::login(
        &state,
        &body.username,
        &body.password,
        &ip,
        &user_agent(&headers),
    )
    .await?;
    Ok(with_cookie(&state, StatusCode::OK, &token, me))
}

async fn logout(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl axum::response::IntoResponse, AppError> {
    accounts::logout(&state, &user).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, clear_cookie(&state))],
    ))
}

async fn preview_invite(
    State(state): State<AppState>,
    JsonIn(body): JsonIn<TokenBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let expires_at = accounts::preview_invite(&state, &body.token).await?;
    Ok(Json(
        serde_json::json!({ "valid": true, "expires_at": expires_at }),
    ))
}

async fn me(State(state): State<AppState>, user: AuthUser) -> Result<Json<MeDto>, AppError> {
    Ok(Json(accounts::load_me(&state, &user).await?))
}

async fn change_password(
    State(state): State<AppState>,
    user: AuthUser,
    JsonIn(body): JsonIn<PasswordBody>,
) -> Result<StatusCode, AppError> {
    accounts::change_password(&state, &user, &body.current_password, &body.new_password).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sessions(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<crate::dto::SessionDto>>, AppError> {
    Ok(Json(accounts::list_sessions(&state, &user).await?))
}

async fn revoke_session(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<StatusCode, AppError> {
    accounts::revoke_session(&state, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout_all(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl axum::response::IntoResponse, AppError> {
    accounts::logout_all(&state, &user).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, clear_cookie(&state))],
    ))
}

async fn create_invite(
    State(state): State<AppState>,
    user: AuthUser,
    JsonIn(body): JsonIn<InviteBody>,
) -> Result<(StatusCode, Json<crate::dto::InviteCreated>), AppError> {
    let created = accounts::create_invite(&state, &user, body.days).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn list_invites(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<crate::dto::InviteDto>>, AppError> {
    Ok(Json(accounts::list_invites(&state, &user).await?))
}

async fn revoke_invite(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<StatusCode, AppError> {
    accounts::revoke_invite(&state, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
