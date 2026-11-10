use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::error::AppErr;
use crate::http::{self, AdminUser, ClientIp};
use crate::repo;
use crate::state::App;
use crate::util;

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/status", get(status))
        .route("/api/users", get(users))
        .route("/api/users/{id}/disable", post(disable_user))
        .route("/api/users/{id}/enable", post(enable_user))
        .route("/api/users/{id}/password", post(reset_password))
        .route("/api/users/{id}/admin", post(grant_admin))
        .route("/api/users/{id}/admin/revoke", post(revoke_admin))
        .route("/api/users/{id}", axum::routing::delete(delete_user))
        .route("/api/invites", get(invites).post(create_invite))
        .route("/api/invites/{id}/revoke", post(revoke_invite))
        .route("/api/conversations", get(conversations))
        .route("/api/audit", get(audit))
        .route("/api/vacuum", post(vacuum))
        .fallback(get(spa))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

async fn spa(uri: axum::http::Uri, headers: HeaderMap) -> Response {
    if uri.path().starts_with("/api") {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "没有找到"}))).into_response();
    }
    crate::assets::serve(uri.path(), "admin.html", &headers)
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
    totp: Option<String>,
}

async fn login(
    State(app): State<Arc<App>>,
    ip: ClientIp,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, AppErr> {
    let body: LoginBody = read_json(body).await?;
    if !app.rate.allow(
        &format!("admin:user:{}", body.username.to_ascii_lowercase()),
        8,
        Duration::from_secs(900),
    ) || !app
        .rate
        .allow(&format!("admin:ip:{}", ip.0), 30, Duration::from_secs(900))
    {
        return Err(AppErr::TooMany);
    }
    let agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let (user, token) = app
        .login(
            &body.username,
            &body.password,
            body.totp.as_deref(),
            true,
            &ip.0,
            agent,
        )
        .await?;
    Ok(http::session_response(&app, "cr_admin", &token, &user))
}

async fn me(user: AdminUser) -> Json<crate::models::UserOut> {
    Json(crate::models::UserOut::from(&user.0.user))
}

async fn logout(State(app): State<Arc<App>>, user: AdminUser) -> Response {
    let _ = app.logout(user.0.session_id, user.0.user.id);
    http::clear_cookie(app.insecure_cookies, "cr_admin")
}

async fn status(
    State(app): State<Arc<App>>,
    _user: AdminUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    let (users, conversations, messages, image_bytes) = app.counts()?;
    Ok(Json(json!({
        "rss_bytes": App::rss_bytes(),
        "uptime_secs": app.started.elapsed().as_secs(),
        "ws_connections": app.hub().connection_count(),
        "users": users,
        "conversations": conversations,
        "messages": messages,
        "image_bytes": image_bytes,
        "disk_bytes": app.disk_bytes(),
    })))
}

async fn users(
    State(app): State<Arc<App>>,
    _user: AdminUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    let users = app.with_db(|conn| repo::list_users(conn))?;
    Ok(Json(json!({"users": users.iter().map(|user| json!({
        "id": user.id,
        "uid": user.uid,
        "username": user.username,
        "display_name": user.display_name,
        "is_admin": user.is_admin,
        "disabled": user.disabled,
        "totp_enabled": user.totp_enabled,
        "created_at": user.created_at,
    })).collect::<Vec<_>>() })))
}

async fn disable_user(
    State(app): State<Arc<App>>,
    user: AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    ensure_other(user.0.user.id, id)?;
    app.with_db(|conn| {
        repo::set_disabled(conn, id, true)?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "user.disable",
            &id.to_string(),
            util::now(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn enable_user(
    State(app): State<Arc<App>>,
    user: AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.with_db(|conn| {
        repo::set_disabled(conn, id, false)?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "user.enable",
            &id.to_string(),
            util::now(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct PasswordBody {
    password: String,
}

async fn reset_password(
    State(app): State<Arc<App>>,
    user: AdminUser,
    Path(id): Path<i64>,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: PasswordBody = read_json(body).await?;
    let password = util::clean_password(&body.password).map_err(AppErr::BadRequest)?;
    let hash = app.hash_pw(password).await?;
    app.with_db(|conn| {
        repo::set_password(conn, id, &hash)?;
        repo::delete_user_sessions(conn, id)?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "user.reset_password",
            &id.to_string(),
            util::now(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn grant_admin(
    State(app): State<Arc<App>>,
    user: AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.with_db(|conn| {
        repo::set_admin(conn, id, true)?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "user.grant_admin",
            &id.to_string(),
            util::now(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_admin(
    State(app): State<Arc<App>>,
    user: AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    ensure_other(user.0.user.id, id)?;
    app.with_db(|conn| {
        repo::set_admin(conn, id, false)?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "user.revoke_admin",
            &id.to_string(),
            util::now(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_user(
    State(app): State<Arc<App>>,
    user: AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    ensure_other(user.0.user.id, id)?;
    let files = app.with_db(|conn| {
        let files = repo::delete_user(conn, id)?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "user.delete",
            &id.to_string(),
            util::now(),
        )?;
        Ok(files)
    })?;
    crate::files::remove_names(&crate::files::images_dir(&app.data_dir), &files);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct InviteBody {
    days: Option<i64>,
}

async fn create_invite(
    State(app): State<Arc<App>>,
    user: AdminUser,
    body: Body,
) -> Result<Json<serde_json::Value>, AppErr> {
    let body: InviteBody = read_json(body).await?;
    let (id, token, expires) = app.with_db(|conn| {
        let created = repo::create_invite(conn, user.0.user.id, body.days.unwrap_or(7))?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "invite.create",
            &created.0.to_string(),
            util::now(),
        )?;
        Ok(created)
    })?;
    Ok(Json(json!({
        "id": id,
        "token": token,
        "expires_at": expires,
        "path": format!("/#/register/{token}"),
        "url": app.invite_link(&token),
    })))
}

async fn invites(
    State(app): State<Arc<App>>,
    _user: AdminUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    let rows = app.with_db(|conn| repo::list_invites(conn))?;
    Ok(Json(json!({"invites": rows.iter().map(|row| json!({
        "id": row.id,
        "created_at": row.created_at,
        "expires_at": row.expires_at,
        "revoked": row.revoked,
        "creator": row.creator,
        "used_by": row.used_by,
        "used_at": row.used_at,
    })).collect::<Vec<_>>() })))
}

async fn revoke_invite(
    State(app): State<Arc<App>>,
    user: AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.with_db(|conn| {
        repo::revoke_invite(conn, id)?;
        repo::audit(
            conn,
            Some(user.0.user.id),
            "invite.revoke",
            &id.to_string(),
            util::now(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn conversations(
    State(app): State<Arc<App>>,
    _user: AdminUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    let rows = app.with_db(|conn| repo::conv_stats(conn))?;
    Ok(Json(json!({"conversations": rows.iter().map(|row| json!({
        "id": row.id,
        "kind": row.kind,
        "retention": row.retention,
        "created_at": row.created_at,
        "members": row.members,
        "messages": row.messages,
        "image_bytes": row.image_bytes,
    })).collect::<Vec<_>>() })))
}

async fn audit(
    State(app): State<Arc<App>>,
    _user: AdminUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    let rows = app.with_db(|conn| repo::list_audit(conn))?;
    Ok(Json(json!({"audit": rows.iter().map(|row| json!({
            "id": row.id,
        "actor_id": row.actor_id,
        "action": row.action,
        "detail": row.detail,
        "created_at": row.created_at,
    })).collect::<Vec<_>>() })))
}

async fn vacuum(State(app): State<Arc<App>>, user: AdminUser) -> Result<StatusCode, AppErr> {
    app.with_db(|conn| {
        repo::vacuum(conn)?;
        repo::audit(conn, Some(user.0.user.id), "vacuum", "", util::now())
    })?;
    Ok(StatusCode::NO_CONTENT)
}

fn ensure_other(actor: i64, target: i64) -> Result<(), AppErr> {
    if actor == target {
        Err(AppErr::BadRequest("不能对自己执行这个操作"))
    } else {
        Ok(())
    }
}

async fn read_json<T: for<'de> serde::Deserialize<'de>>(body: Body) -> Result<T, AppErr> {
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|_| AppErr::BadRequest("请求过大"))?;
    serde_json::from_slice(&bytes).map_err(|_| AppErr::BadRequest("请求格式错误"))
}
