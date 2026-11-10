use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{
    ConnectInfo, DefaultBodyLimit, FromRequestParts, Multipart, Path, State, WebSocketUpgrade,
};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tokio::time::{interval, Duration};

use crate::assets;
use crate::error::AppErr;
use crate::models::SessionUser;
use crate::state::App;
use crate::util::{self, cookie};

const APP_COOKIE: &str = "cr_session";
const ADMIN_COOKIE: &str = "cr_admin";

pub struct ClientIp(pub String);

impl FromRequestParts<Arc<App>> for ClientIp {
    type Rejection = std::convert::Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<App>,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0);
        let headers = parts.headers.clone();
        async move { Ok(ClientIp(client_ip(peer, &headers))) }
    }
}

pub struct AppUser(pub SessionUser);

impl FromRequestParts<Arc<App>> for AppUser {
    type Rejection = AppErr;

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<App>,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let headers = parts.headers.clone();
        let state = Arc::clone(state);
        async move {
            let token = cookie(&headers, APP_COOKIE).ok_or(AppErr::Unauthorized("请先登录"))?;
            let session = state
                .with_db(|conn| crate::repo::session_user(conn, &token, "app", util::now()))?;
            Ok(AppUser(session))
        }
    }
}

pub struct AdminUser(pub SessionUser);

impl FromRequestParts<Arc<App>> for AdminUser {
    type Rejection = AppErr;

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<App>,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let headers = parts.headers.clone();
        let state = Arc::clone(state);
        async move {
            let token = cookie(&headers, ADMIN_COOKIE).ok_or(AppErr::Unauthorized("请先登录"))?;
            let session = state
                .with_db(|conn| crate::repo::session_user(conn, &token, "admin", util::now()))?;
            Ok(AdminUser(session))
        }
    }
}

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/me/password", post(change_password))
        .route("/api/me/totp/begin", post(totp_begin))
        .route("/api/me/totp/enable", post(totp_enable))
        .route("/api/me/totp/disable", post(totp_disable))
        .route("/api/me/sessions", get(sessions))
        .route(
            "/api/me/sessions/{id}",
            axum::routing::delete(revoke_session),
        )
        .route("/api/users/search", post(search))
        .route("/api/friends", get(friends))
        .route("/api/friends/requests", get(requests).post(friend_request))
        .route("/api/friends/requests/{id}/accept", post(friend_accept))
        .route("/api/friends/requests/{id}/reject", post(friend_reject))
        .route("/api/friends/{id}", axum::routing::delete(friend_delete))
        .route("/api/conversations", get(conversations))
        .route("/api/conversations/dm", post(create_dm))
        .route("/api/conversations/group", post(create_group))
        .route(
            "/api/conversations/{id}",
            get(conversation).patch(update_conversation),
        )
        .route(
            "/api/conversations/{id}/messages",
            get(messages).post(post_message),
        )
        .route("/api/conversations/{id}/read", post(mark_read))
        .route("/api/conversations/{id}/members", post(add_member))
        .route(
            "/api/conversations/{id}/members/{user_id}",
            axum::routing::delete(kick_member),
        )
        .route("/api/conversations/{id}/transfer", post(transfer))
        .route("/api/conversations/{id}/leave", post(leave))
        .route("/api/messages/{id}/recall", post(recall))
        .route("/api/messages/{id}/hide", post(hide))
        .route("/api/images/{id}", get(image))
        .route("/api/images/{id}/thumb", get(thumb))
        .route("/api/push/vapid", get(vapid))
        .route(
            "/api/push/subscribe",
            post(push_subscribe).delete(push_unsubscribe),
        )
        .route("/api/ws", get(ws))
        .route("/api/conversations/{id}/images", post(upload_image))
        .fallback(get(spa))
}

pub fn wrap(router: Router<Arc<App>>) -> Router<Arc<App>> {
    router
        .layer(DefaultBodyLimit::max(26 * 1024 * 1024))
        .layer(middleware::from_fn(guard))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn(access_log))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

async fn spa(uri: axum::http::Uri, headers: HeaderMap) -> Response {
    if uri.path().starts_with("/api") {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "没有找到"}))).into_response();
    }
    assets::serve(uri.path(), "index.html", &headers)
}

#[derive(Deserialize)]
struct RegisterBody {
    token: String,
    username: String,
    password: String,
    display_name: String,
}

async fn register(
    State(app): State<Arc<App>>,
    ip: ClientIp,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, AppErr> {
    let body: RegisterBody = read_json(body).await?;
    let (user, token) = app
        .register(
            &body.token,
            &body.username,
            &body.password,
            &body.display_name,
            &ip.0,
            user_agent(&headers),
        )
        .await?;
    Ok(session_response(&app, APP_COOKIE, &token, &user))
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
        &format!("login:user:{}", body.username.to_ascii_lowercase()),
        8,
        Duration::from_secs(900),
    ) || !app
        .rate
        .allow(&format!("login:ip:{}", ip.0), 30, Duration::from_secs(900))
    {
        return Err(AppErr::TooMany);
    }
    let (user, token) = app
        .login(
            &body.username,
            &body.password,
            body.totp.as_deref(),
            false,
            &ip.0,
            user_agent(&headers),
        )
        .await?;
    Ok(session_response(&app, APP_COOKIE, &token, &user))
}

async fn logout(State(app): State<Arc<App>>, user: AppUser) -> Response {
    let _ = app.logout(user.0.session_id, user.0.user.id);
    clear_cookie(app.insecure_cookies, APP_COOKIE)
}

async fn me(user: AppUser) -> Json<crate::models::UserOut> {
    Json(crate::models::UserOut::from(&user.0.user))
}

#[derive(Deserialize)]
struct PasswordBody {
    old_password: String,
    new_password: String,
}

async fn change_password(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: PasswordBody = read_json(body).await?;
    app.change_password(&user.0, &body.old_password, &body.new_password)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn totp_begin(
    State(app): State<Arc<App>>,
    user: AppUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    let (secret, url) = app.totp_begin(&user.0.user)?;
    Ok(Json(json!({"secret": secret, "otpauth": url})))
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

async fn totp_enable(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: CodeBody = read_json(body).await?;
    app.totp_enable(&user.0.user, &body.code)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct DisableTotpBody {
    password: String,
    code: String,
}

async fn totp_disable(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: DisableTotpBody = read_json(body).await?;
    app.totp_disable(&user.0.user, &body.password, &body.code)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sessions(
    State(app): State<Arc<App>>,
    user: AppUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    let rows = app.sessions(&user.0)?;
    Ok(Json(json!({"sessions": rows.iter().map(|row| json!({
        "id": row.id,
        "created_at": row.created_at,
        "last_seen": row.last_seen,
        "ip": row.ip,
        "user_agent": row.user_agent,
        "current": row.current,
    })).collect::<Vec<_>>() })))
}

async fn revoke_session(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.revoke_session(user.0.user.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SearchBody {
    q: String,
}

async fn search(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<Json<serde_json::Value>, AppErr> {
    let body: SearchBody = read_json(body).await?;
    if !app.rate.allow(
        &format!("search:{}", user.0.user.id),
        20,
        Duration::from_secs(60),
    ) {
        return Err(AppErr::TooMany);
    }
    Ok(Json(json!({"users": app.search(user.0.user.id, &body.q)?})))
}

async fn friends(
    State(app): State<Arc<App>>,
    user: AppUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    Ok(Json(json!({"friends": app.friends(user.0.user.id)?})))
}

async fn requests(
    State(app): State<Arc<App>>,
    user: AppUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    Ok(Json(json!({"requests": app.requests(user.0.user.id)?})))
}

#[derive(Deserialize)]
struct FriendBody {
    user_id: i64,
}

async fn friend_request(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<Json<serde_json::Value>, AppErr> {
    let body: FriendBody = read_json(body).await?;
    if !app.rate.allow(
        &format!("friend:{}", user.0.user.id),
        20,
        Duration::from_secs(3600),
    ) {
        return Err(AppErr::TooMany);
    }
    let accepted = app.friend_request(user.0.user.id, body.user_id)?;
    Ok(Json(json!({"accepted": accepted})))
}

async fn friend_accept(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.friend_accept(user.0.user.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn friend_reject(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.friend_reject(user.0.user.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn friend_delete(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.friend_delete(user.0.user.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn conversations(
    State(app): State<Arc<App>>,
    user: AppUser,
) -> Result<Json<serde_json::Value>, AppErr> {
    Ok(Json(
        json!({"conversations": app.conversations(user.0.user.id)?}),
    ))
}

async fn conversation(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppErr> {
    Ok(Json(json!(app.conversation(user.0.user.id, id)?)))
}

async fn create_dm(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<Json<serde_json::Value>, AppErr> {
    let body: FriendBody = read_json(body).await?;
    let id = app.create_dm(user.0.user.id, body.user_id)?;
    Ok(Json(json!({"id": id})))
}

#[derive(Deserialize)]
struct GroupBody {
    name: String,
    member_ids: Vec<i64>,
}

async fn create_group(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<Json<serde_json::Value>, AppErr> {
    let body: GroupBody = read_json(body).await?;
    if body.member_ids.len() > 99 {
        return Err(AppErr::BadRequest("群人数不能超过 100"));
    }
    let id = app.create_group(user.0.user.id, &body.name, &body.member_ids)?;
    Ok(Json(json!({"id": id})))
}

#[derive(Deserialize)]
struct UpdateConversation {
    name: Option<String>,
    retention: Option<i64>,
}

async fn update_conversation(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: UpdateConversation = read_json(body).await?;
    if let Some(retention) = body.retention {
        app.set_retention(user.0.user.id, id, retention)?;
    }
    if let Some(name) = body.name {
        app.rename_group(user.0.user.id, id, &name)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ListQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

async fn messages(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
    axum::extract::Query(query): axum::extract::Query<ListQuery>,
) -> Result<Json<serde_json::Value>, AppErr> {
    Ok(Json(
        json!({"messages": app.messages(user.0.user.id, id, query.before, query.limit.unwrap_or(50))?}),
    ))
}

#[derive(Deserialize)]
struct TextBody {
    text: String,
}

async fn post_message(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
    body: Body,
) -> Result<Json<serde_json::Value>, AppErr> {
    let body: TextBody = read_json(body).await?;
    if !app.rate.allow(
        &format!("msg:{}", user.0.user.id),
        120,
        Duration::from_secs(60),
    ) {
        return Err(AppErr::TooMany);
    }
    Ok(Json(json!(app.post_text(
        user.0.user.id,
        id,
        &body.text
    )?)))
}

async fn upload_image(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppErr> {
    if !app.rate.allow(
        &format!("img:{}", user.0.user.id),
        20,
        Duration::from_secs(60),
    ) {
        return Err(AppErr::TooMany);
    }
    let mut file = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppErr::BadRequest("上传无效"))?
    {
        if field.name() == Some("file") {
            let bytes = field
                .bytes()
                .await
                .map_err(|_| AppErr::BadRequest("上传无效"))?;
            file = Some(bytes);
            break;
        }
    }
    let Some(file) = file else {
        return Err(AppErr::BadRequest("缺少文件"));
    };
    if file.len() > util::MAX_UPLOAD {
        return Err(AppErr::BadRequest("图片不能超过 25MB"));
    }
    Ok(Json(json!(
        app.post_image(user.0.user.id, id, file.to_vec()).await?
    )))
}

#[derive(Deserialize)]
struct ReadBody {
    message_id: i64,
}

async fn mark_read(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: ReadBody = read_json(body).await?;
    app.mark_read(user.0.user.id, id, body.message_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_member(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: FriendBody = read_json(body).await?;
    app.add_member(user.0.user.id, id, body.user_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn kick_member(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path((id, target)): Path<(i64, i64)>,
) -> Result<StatusCode, AppErr> {
    app.kick(user.0.user.id, id, target)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn transfer(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: FriendBody = read_json(body).await?;
    app.transfer(user.0.user.id, id, body.user_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.leave(user.0.user.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn recall(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppErr> {
    Ok(Json(json!(app.recall(user.0.user.id, id)?)))
}

async fn hide(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppErr> {
    app.hide(user.0.user.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn image(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<Response, AppErr> {
    image_response(&app, user.0.user.id, id, false)
}

async fn thumb(
    State(app): State<Arc<App>>,
    user: AppUser,
    Path(id): Path<i64>,
) -> Result<Response, AppErr> {
    image_response(&app, user.0.user.id, id, true)
}

fn image_response(app: &App, user: i64, id: i64, thumb: bool) -> Result<Response, AppErr> {
    let bytes = app.read_image(user, id, thumb)?;
    let mut response = Body::from(bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    Ok(response)
}

async fn vapid(State(app): State<Arc<App>>, _user: AppUser) -> Json<serde_json::Value> {
    Json(json!({"publicKey": app.vapid_public()}))
}

#[derive(Deserialize)]
struct PushBody {
    endpoint: String,
    keys: PushKeys,
}

#[derive(Deserialize)]
struct PushKeys {
    p256dh: String,
    auth: String,
}

async fn push_subscribe(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: PushBody = read_json(body).await?;
    app.save_push(
        user.0.user.id,
        &body.endpoint,
        &body.keys.p256dh,
        &body.keys.auth,
    )?;
    Ok(StatusCode::NO_CONTENT)
}

async fn push_unsubscribe(
    State(app): State<Arc<App>>,
    user: AppUser,
    body: Body,
) -> Result<StatusCode, AppErr> {
    let body: PushBody = read_json(body).await?;
    app.delete_push(user.0.user.id, &body.endpoint)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ws(
    user: AppUser,
    State(app): State<Arc<App>>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    let user_id = user.0.user.id;
    upgrade
        .max_message_size(4096)
        .on_upgrade(move |socket| ws_loop(socket, app, user_id))
}

#[allow(clippy::collapsible_match)]
async fn ws_loop(mut socket: WebSocket, app: Arc<App>, user_id: i64) {
    let Some(sub) = app.hub().subscribe(user_id, 5) else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    let mut rx = sub.rx;
    let mut ticker = interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if socket.send(Message::Ping(vec![1].into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Text(text))) => {
                        if text.contains("ping")
                            && socket
                                .send(Message::Text("{\"type\":\"pong\"}".into()))
                                .await
                                .is_err()
                        {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            event = rx.recv() => {
                match event {
                    Some(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
    app.hub().unsubscribe(user_id, sub.id);
}

pub fn session_response(
    app: &App,
    name: &str,
    token: &str,
    user: &crate::models::User,
) -> Response {
    let mut response = Json(crate::models::UserOut::from(user)).into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie_value(
        name,
        token,
        util::SESSION_SECS,
        !app.insecure_cookies,
    )) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

pub fn clear_cookie(insecure: bool, name: &str) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie_value(name, "", 0, !insecure)) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

fn cookie_value(name: &str, token: &str, max_age: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{name}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{secure}")
}

async fn read_json<T: for<'de> Deserialize<'de>>(body: Body) -> Result<T, AppErr> {
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|_| AppErr::BadRequest("请求过大"))?;
    serde_json::from_slice(&bytes).map_err(|_| AppErr::BadRequest("请求格式错误"))
}

fn user_agent(headers: &HeaderMap) -> &str {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

fn client_ip(peer: Option<SocketAddr>, headers: &HeaderMap) -> String {
    let peer_ip = peer.map(|addr| addr.ip());
    if peer_ip.is_some_and(|ip| ip.is_loopback()) {
        if let Some(value) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        {
            if let Some(ip) = value
                .split(',')
                .filter_map(|part| part.trim().parse::<std::net::IpAddr>().ok())
                .next_back()
            {
                return ip.to_string();
            }
        }
    }
    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "local".into())
}

async fn guard(req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    if matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) && !origin_ok(req.headers())
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "跨站请求被拒绝"})),
        )
            .into_response();
    }
    let big = method == Method::POST && path.ends_with("/images");
    let max: usize = if big {
        util::MAX_UPLOAD + 1024 * 1024
    } else {
        64 * 1024
    };
    if let Some(len) = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    {
        if len > max {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"error": "请求过大"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

fn origin_ok(headers: &HeaderMap) -> bool {
    if let Some(site) = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
    {
        if site == "cross-site" {
            return false;
        }
    }
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin.split("://").nth(1) == Some(host)
}

async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' blob:; style-src 'self'; style-src-attr 'unsafe-inline'; script-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
}

async fn access_log(req: Request<Body>, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let response = next.run(req).await;
    tracing::info!(%method, %path, status = %response.status(), "请求");
    response
}
