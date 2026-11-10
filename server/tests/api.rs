use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chat_server::domain::auth::{create_invite_for, provision_admin};
use chat_server::janitor;
use chat_server::{router, AppState, Config};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

struct Harness {
    _dir: TempDir,
    state: AppState,
    app: axum::Router,
}

async fn harness() -> Harness {
    let dir = TempDir::new().unwrap();
    let state = AppState::boot(Config::for_test(dir.path())).await.unwrap();
    let app = router(state.clone());
    Harness {
        _dir: dir,
        state,
        app,
    }
}

struct Response {
    status: StatusCode,
    body: Value,
    bytes: Vec<u8>,
    cookie: Option<String>,
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    body: Option<Value>,
) -> Response {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(origin) = origin {
        builder = builder.header("origin", origin);
    }
    let request = if let Some(body) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap().to_string());
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Response {
        status,
        body,
        bytes,
        cookie,
    }
}

async fn upload(app: &axum::Router, cookie: &str, png: &[u8]) -> Response {
    let boundary = "----chat";
    let mut raw = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    raw.extend_from_slice(png);
    raw.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
    let request = Request::builder()
        .method("POST")
        .uri("/api/uploads")
        .header("cookie", cookie)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(raw))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Response {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        bytes,
        cookie: None,
    }
}

fn png_bytes() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(6, 4, image::Rgba([9, 8, 7, 255]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    cursor.into_inner()
}

async fn register(app: &axum::Router, token: &str, username: &str) -> Response {
    call(
        app,
        "POST",
        "/api/auth/register",
        None,
        None,
        Some(json!({
            "token": token,
            "username": username,
            "password": "correct horse"
        })),
    )
    .await
}

#[tokio::test]
async fn invite_is_single_use_and_search_is_exact() {
    let h = harness().await;
    let bad = call(
        &h.app,
        "POST",
        "/api/auth/register",
        None,
        None,
        Some(json!({"token":"nope","username":"bob","password":"correct horse"})),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    let weak = call(
        &h.app,
        "POST",
        "/api/auth/register",
        None,
        None,
        Some(json!({"token":"nope","username":"bob","password":"password123"})),
    )
    .await;
    assert_eq!(weak.status, StatusCode::BAD_REQUEST);

    let admin = provision_admin(&h.state, "ada", "correct horse")
        .await
        .unwrap();
    let invite = create_invite_for(&h.state, admin, 7).await.unwrap();
    assert!(invite.url.contains("/invite#"));
    let bob = register(&h.app, &invite.token, "bob").await;
    assert_eq!(bob.status, StatusCode::CREATED);
    assert!(bob.cookie.unwrap().starts_with("session="));
    let again = register(&h.app, &invite.token, "cara").await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);

    let invite2 = create_invite_for(&h.state, admin, 7).await.unwrap();
    let cara = register(&h.app, &invite2.token, "cara").await;
    let cara_cookie = cara.cookie.unwrap();
    let found = call(
        &h.app,
        "GET",
        "/api/users/search?q=ada",
        Some(&cara_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(found.body["users"][0]["username"], "ada");
    let missing = call(
        &h.app,
        "GET",
        "/api/users/search?q=ad",
        Some(&cara_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(missing.body["users"].as_array().unwrap().len(), 0);
    let uid = found.body["users"][0]["uid"].as_str().unwrap();
    let by_uid = call(
        &h.app,
        "GET",
        &format!("/api/users/search?q={uid}"),
        Some(&cara_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(by_uid.body["users"][0]["uid"], uid);
    let prefix = &uid[..6];
    let by_prefix = call(
        &h.app,
        "GET",
        &format!("/api/users/search?q={prefix}"),
        Some(&cara_cookie),
        None,
        None,
    )
    .await;
    assert!(by_prefix.body["users"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn friends_groups_messages_and_encryption() {
    let h = harness().await;
    let admin = provision_admin(&h.state, "ada", "correct horse")
        .await
        .unwrap();
    let bob_invite = create_invite_for(&h.state, admin, 7).await.unwrap();
    let cara_invite = create_invite_for(&h.state, admin, 7).await.unwrap();
    let bob = register(&h.app, &bob_invite.token, "bob").await;
    let cara = register(&h.app, &cara_invite.token, "cara").await;
    let bob_cookie = bob.cookie.unwrap();
    let cara_cookie = cara.cookie.unwrap();
    let ada = call(
        &h.app,
        "GET",
        "/api/users/search?q=ada",
        Some(&bob_cookie),
        None,
        None,
    )
    .await;
    let ada_id = ada.body["users"][0]["id"].as_i64().unwrap();
    let cara_user = call(
        &h.app,
        "GET",
        "/api/users/search?q=cara",
        Some(&bob_cookie),
        None,
        None,
    )
    .await;
    let cara_id = cara_user.body["users"][0]["id"].as_i64().unwrap();

    let blocked = call(
        &h.app,
        "POST",
        "/api/conversations",
        Some(&bob_cookie),
        None,
        Some(json!({"kind":"direct","user_id": cara_id})),
    )
    .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);

    let request = call(
        &h.app,
        "POST",
        "/api/friends/requests",
        Some(&bob_cookie),
        None,
        Some(json!({"user_id": cara_id})),
    )
    .await;
    assert_eq!(request.status, StatusCode::CREATED);
    let request_id = request.body["id"].as_i64().unwrap();
    let accepted = call(
        &h.app,
        "POST",
        &format!("/api/friends/requests/{request_id}/accept"),
        Some(&cara_cookie),
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::NO_CONTENT);

    let direct = call(
        &h.app,
        "POST",
        "/api/conversations",
        Some(&bob_cookie),
        None,
        Some(json!({"kind":"direct","user_id": cara_id})),
    )
    .await;
    assert_eq!(direct.status, StatusCode::CREATED);
    let conv_id = direct.body["id"].as_i64().unwrap();
    let sent = call(
        &h.app,
        "POST",
        &format!("/api/conversations/{conv_id}/messages"),
        Some(&bob_cookie),
        None,
        Some(json!({"kind":"text","body":"hello secret"})),
    )
    .await;
    assert_eq!(sent.status, StatusCode::CREATED);
    let message_id = sent.body["id"].as_i64().unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("SELECT body_ct FROM messages WHERE id = ?")
        .bind(message_id)
        .fetch_one(&h.state.db)
        .await
        .unwrap();
    assert!(!stored.windows(5).any(|window| window == b"hello"));
    let nonce: Vec<u8> = sqlx::query_scalar("SELECT nonce FROM messages WHERE id = ?")
        .bind(message_id)
        .fetch_one(&h.state.db)
        .await
        .unwrap();
    assert_eq!(
        h.state.crypto.open(&nonce, &stored).unwrap(),
        b"hello secret"
    );

    let deleted = call(
        &h.app,
        "DELETE",
        &format!("/api/conversations/{conv_id}/messages/{message_id}"),
        Some(&cara_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::FORBIDDEN);
    let removed = call(
        &h.app,
        "DELETE",
        &format!("/api/conversations/{conv_id}/messages/{message_id}"),
        Some(&bob_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);

    let stranger_group = call(
        &h.app,
        "POST",
        "/api/conversations",
        Some(&bob_cookie),
        None,
        Some(json!({"kind":"group","name":"房间","member_ids":[ada_id]})),
    )
    .await;
    assert_eq!(stranger_group.status, StatusCode::FORBIDDEN);
    let group = call(
        &h.app,
        "POST",
        "/api/conversations",
        Some(&bob_cookie),
        None,
        Some(json!({"kind":"group","name":"房间","member_ids":[cara_id]})),
    )
    .await;
    assert_eq!(group.status, StatusCode::CREATED);
    let group_id = group.body["id"].as_i64().unwrap();
    let bad_retention = call(
        &h.app,
        "PATCH",
        &format!("/api/conversations/{group_id}"),
        Some(&bob_cookie),
        None,
        Some(json!({"retention_secs": 999999})),
    )
    .await;
    assert_eq!(bad_retention.status, StatusCode::BAD_REQUEST);
    let retention = call(
        &h.app,
        "PATCH",
        &format!("/api/conversations/{group_id}"),
        Some(&bob_cookie),
        None,
        Some(json!({"retention_secs": 3600})),
    )
    .await;
    assert_eq!(retention.status, StatusCode::OK);
    let kick = call(
        &h.app,
        "DELETE",
        &format!("/api/conversations/{group_id}/members/{cara_id}"),
        Some(&cara_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(kick.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn images_are_reencoded_and_expired_messages_disappear() {
    let h = harness().await;
    let admin = provision_admin(&h.state, "ada", "correct horse")
        .await
        .unwrap();
    let invite = create_invite_for(&h.state, admin, 7).await.unwrap();
    let bob = register(&h.app, &invite.token, "bob").await;
    let cookie = bob.cookie.unwrap();
    let ada = call(
        &h.app,
        "GET",
        "/api/users/search?q=ada",
        Some(&cookie),
        None,
        None,
    )
    .await;
    let ada_id = ada.body["users"][0]["id"].as_i64().unwrap();
    let request = call(
        &h.app,
        "POST",
        "/api/friends/requests",
        Some(&cookie),
        None,
        Some(json!({"user_id": ada_id})),
    )
    .await;
    // ada has no session cookie; accept through a second login
    let ada_login = call(
        &h.app,
        "POST",
        "/api/auth/login",
        None,
        None,
        Some(json!({"username":"ada","password":"correct horse"})),
    )
    .await;
    let ada_cookie = ada_login.cookie.unwrap();
    let request_id = request.body["id"].as_i64().unwrap();
    call(
        &h.app,
        "POST",
        &format!("/api/friends/requests/{request_id}/accept"),
        Some(&ada_cookie),
        None,
        Some(json!({})),
    )
    .await;
    let direct = call(
        &h.app,
        "POST",
        "/api/conversations",
        Some(&cookie),
        None,
        Some(json!({"kind":"direct","user_id": ada_id})),
    )
    .await;
    let conv_id = direct.body["id"].as_i64().unwrap();
    let uploaded = upload(&h.app, &cookie, &png_bytes()).await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.body);
    let attachment_id = uploaded.body["id"].as_i64().unwrap();
    let sent = call(
        &h.app,
        "POST",
        &format!("/api/conversations/{conv_id}/messages"),
        Some(&cookie),
        None,
        Some(json!({"kind":"image","attachment_id": attachment_id})),
    )
    .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{}", sent.body);
    let message_id = sent.body["id"].as_i64().unwrap();
    let image = call(
        &h.app,
        "GET",
        &format!("/api/attachments/{attachment_id}"),
        Some(&ada_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(image.status, StatusCode::OK);
    assert!(image.bytes.starts_with(b"RIFF"));
    assert_eq!(&image.bytes[8..12], b"WEBP");
    let outsider = register(
        &h.app,
        &create_invite_for(&h.state, admin, 7).await.unwrap().token,
        "erin",
    )
    .await;
    let hidden = call(
        &h.app,
        "GET",
        &format!("/api/attachments/{attachment_id}"),
        Some(&outsider.cookie.unwrap()),
        None,
        None,
    )
    .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);

    sqlx::query("UPDATE messages SET expires_at = 1 WHERE id = ?")
        .bind(message_id)
        .execute(&h.state.db)
        .await
        .unwrap();
    let stats = janitor::cleanup(&h.state).await.unwrap();
    assert!(stats.messages_deleted >= 1);
    assert!(stats.attachments_deleted >= 1);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = ?")
        .bind(message_id)
        .fetch_one(&h.state.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn login_lockout_and_cross_site_rejection() {
    let h = harness().await;
    provision_admin(&h.state, "ada", "correct horse")
        .await
        .unwrap();
    let evil = call(
        &h.app,
        "POST",
        "/api/auth/login",
        None,
        Some("https://evil.example"),
        Some(json!({"username":"ada","password":"correct horse"})),
    )
    .await;
    assert_eq!(evil.status, StatusCode::FORBIDDEN);
    for _ in 0..5 {
        let failed = call(
            &h.app,
            "POST",
            "/api/auth/login",
            None,
            None,
            Some(json!({"username":"ada","password":"wrong horse"})),
        )
        .await;
        assert_eq!(failed.status, StatusCode::UNAUTHORIZED);
    }
    let locked = call(
        &h.app,
        "POST",
        "/api/auth/login",
        None,
        None,
        Some(json!({"username":"ada","password":"correct horse"})),
    )
    .await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn websocket_receives_messages() {
    let h = harness().await;
    let admin = provision_admin(&h.state, "ada", "correct horse")
        .await
        .unwrap();
    let bob = register(
        &h.app,
        &create_invite_for(&h.state, admin, 7).await.unwrap().token,
        "bob",
    )
    .await;
    let bob_cookie = bob.cookie.clone().unwrap();
    let ada_login = call(
        &h.app,
        "POST",
        "/api/auth/login",
        None,
        None,
        Some(json!({"username":"ada","password":"correct horse"})),
    )
    .await;
    let ada_cookie = ada_login.cookie.unwrap();
    let ada = call(
        &h.app,
        "GET",
        "/api/users/search?q=ada",
        Some(&bob_cookie),
        None,
        None,
    )
    .await;
    let ada_id = ada.body["users"][0]["id"].as_i64().unwrap();
    let request = call(
        &h.app,
        "POST",
        "/api/friends/requests",
        Some(&bob_cookie),
        None,
        Some(json!({"user_id": ada_id})),
    )
    .await;
    call(
        &h.app,
        "POST",
        &format!(
            "/api/friends/requests/{}/accept",
            request.body["id"].as_i64().unwrap()
        ),
        Some(&ada_cookie),
        None,
        Some(json!({})),
    )
    .await;
    let direct = call(
        &h.app,
        "POST",
        "/api/conversations",
        Some(&ada_cookie),
        None,
        Some(json!({"kind":"direct","user_id": bob.body["id"].as_i64().unwrap()})),
    )
    .await;
    let conv_id = direct.body["id"].as_i64().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(h.state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("cookie", bob_cookie.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let ready = socket_text(&mut socket).await;
    assert_eq!(ready["type"], "ready");

    call(
        &h.app,
        "POST",
        &format!("/api/conversations/{conv_id}/messages"),
        Some(&ada_cookie),
        None,
        Some(json!({"kind":"text","body":"from ada"})),
    )
    .await;
    let event = socket_text(&mut socket).await;
    assert_eq!(event["type"], "message");
    assert_eq!(event["message"]["body"], "from ada");

    use futures_util::SinkExt;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"type":"send","conv_id": conv_id, "kind":"text","body":"from bob","client_msg_id":"c1"}).to_string().into(),
        ))
        .await
        .unwrap();
    let ack = socket_text(&mut socket).await;
    assert_eq!(ack["message"]["body"], "from bob");
    assert_eq!(ack["message"]["client_msg_id"], "c1");
}

async fn socket_text(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    use futures_util::StreamExt;
    let message = socket.next().await.unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}
