use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::error::StartupError;
use crate::state::App;
use crate::util::{RET_HOUR, RET_MONTH};

fn open(dir: &std::path::Path, passphrase: &str) -> Arc<App> {
    Arc::new(App::open(dir, passphrase, "https://chat.example").unwrap())
}

#[test]
fn wrong_key_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    open(dir.path(), "correct-passphrase");
    match App::open(dir.path(), "different-passphrase", "https://chat.example") {
        Err(StartupError::BadKey) => {}
        Ok(_) => panic!("错误的主密钥不应该能打开数据库"),
        Err(err) => panic!("期望密钥错误，实际是 {err}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn invites_friends_messages_and_retention() {
    let dir = tempfile::tempdir().unwrap();
    let app = open(dir.path(), "correct-passphrase");
    let (invite, _token, _) = app
        .with_db(|conn| crate::repo::create_invite(conn, 0, 7))
        .unwrap();
    let token = app
        .with_db(|conn| {
            let token = crate::util::b64(&crate::util::random_bytes::<32>());
            conn.execute(
                "UPDATE invites SET token_hash=?1 WHERE id=?2",
                rusqlite::params![crate::util::sha256(token.as_bytes()).to_vec(), invite],
            )?;
            Ok::<_, crate::error::AppErr>(token)
        })
        .unwrap();
    // The update above replaced the original hash, so use `token`.
    let (alice, _) = app
        .register(
            &token,
            "Alice",
            "password-alice",
            "爱丽丝",
            "127.0.0.1",
            "test",
        )
        .await
        .unwrap();
    assert_eq!(alice.uid.len(), 7);
    assert!(alice.uid.chars().all(|c| c.is_ascii_digit()));
    assert!(app
        .register(
            &token,
            "Bob",
            "password-bob-ok",
            "鲍勃",
            "127.0.0.1",
            "test"
        )
        .await
        .is_err());

    let token2 = make_invite(&app);
    app.with_db(|conn| {
        conn.execute(
            "UPDATE invites SET revoked=1 WHERE token_hash=?1",
            [crate::util::sha256(token2.as_bytes()).to_vec()],
        )?;
        Ok::<_, crate::error::AppErr>(())
    })
    .unwrap();
    assert!(app
        .register(
            &token2,
            "Bob",
            "password-bob-ok",
            "鲍勃",
            "127.0.0.1",
            "test"
        )
        .await
        .is_err());

    let token3 = make_invite(&app);
    app.with_db(|conn| {
        conn.execute(
            "UPDATE invites SET expires_at=1 WHERE token_hash=?1",
            [crate::util::sha256(token3.as_bytes()).to_vec()],
        )?;
        Ok::<_, crate::error::AppErr>(())
    })
    .unwrap();
    assert!(app
        .register(
            &token3,
            "Bob",
            "password-bob-ok",
            "鲍勃",
            "127.0.0.1",
            "test"
        )
        .await
        .is_err());

    let token4 = make_invite(&app);
    let (bob, _) = app
        .register(
            &token4,
            "Bob",
            "password-bob-ok",
            "鲍勃",
            "127.0.0.1",
            "test",
        )
        .await
        .unwrap();
    assert!(app
        .register(
            &make_invite(&app),
            "bob",
            "password-bob-two",
            "另一个",
            "127.0.0.1",
            "test"
        )
        .await
        .is_err());
    assert_ne!(alice.uid, bob.uid);
    assert!(app.friends(alice.id).unwrap().is_empty());

    let found = app.search(alice.id, "bob").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, bob.id);
    assert!(app.search(alice.id, "bo").unwrap().is_empty());
    assert_eq!(app.search(alice.id, &bob.uid).unwrap()[0].id, bob.id);

    assert!(app.create_dm(alice.id, bob.id).is_err());
    assert!(!app.friend_request(alice.id, bob.id).unwrap());
    let incoming = app.requests(bob.id).unwrap();
    assert_eq!(incoming.len(), 1);
    app.friend_accept(bob.id, incoming[0].id).unwrap();
    let friends = app.friends(alice.id).unwrap();
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].id, bob.id);
    let dm = app.create_dm(alice.id, bob.id).unwrap();
    assert_eq!(app.create_dm(bob.id, alice.id).unwrap(), dm);

    let sent = app.post_text(alice.id, dm, "你好").unwrap();
    assert_eq!(sent.body.as_deref(), Some("你好"));
    let recalled = app.recall(alice.id, sent.id).unwrap();
    assert!(recalled.recalled);
    assert!(recalled.body.is_none());
    assert!(app
        .messages(bob.id, dm, None, 20)
        .unwrap()
        .iter()
        .any(|message| message.recalled));

    let hidden = app.post_text(bob.id, dm, "只给你看").unwrap();
    app.hide(alice.id, hidden.id).unwrap();
    assert!(app
        .messages(alice.id, dm, None, 20)
        .unwrap()
        .iter()
        .all(|message| message.id != hidden.id));
    assert!(app
        .messages(bob.id, dm, None, 20)
        .unwrap()
        .iter()
        .any(|message| message.id == hidden.id));

    app.set_retention(alice.id, dm, RET_HOUR).unwrap();
    let expiring = app.post_text(alice.id, dm, "很快消失").unwrap();
    app.with_db(|conn| crate::repo::force_expiry(conn, expiring.id, 1))
        .unwrap();
    app.maintenance();
    assert!(app
        .messages(alice.id, dm, None, 50)
        .unwrap()
        .iter()
        .all(|message| message.id != expiring.id));

    let group = app.create_group(alice.id, "小组", &[bob.id]).unwrap();
    assert!(app.rename_group(bob.id, group, "不行").is_err());
    app.rename_group(alice.id, group, "新名字").unwrap();
    assert_eq!(app.conversation(bob.id, group).unwrap().title, "新名字");
    assert_eq!(
        app.conversation(alice.id, group).unwrap().retention,
        RET_MONTH
    );

    let jpeg = tiny_jpeg();
    let image = app.post_image(alice.id, dm, jpeg.clone()).await.unwrap();
    let stored = app
        .read_image(bob.id, image.image_id.unwrap(), false)
        .unwrap();
    assert!(stored.starts_with(&[0xff, 0xd8]));
    assert_ne!(stored, jpeg);
    let names = app
        .with_db(|conn| crate::repo::referenced_images(conn))
        .unwrap();
    let dir = crate::files::images_dir(dir.path());
    for name in names {
        let disk = std::fs::read(dir.join(&name)).unwrap();
        assert!(disk.len() >= 24, "加密后的图片不应过短");
        assert_ne!(disk, stored);
        assert!(
            image::load_from_memory(&disk).is_err(),
            "磁盘上不应保存未加密的 JPEG"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn http_login_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut opened = App::open(dir.path(), "correct-passphrase", "https://chat.example").unwrap();
    opened.insecure_cookies = true;
    let app = Arc::new(opened);
    let token = make_invite(&app);
    let router =
        crate::http::wrap(Arc::clone(&app), crate::http::router()).with_state(Arc::clone(&app));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/register")
                .header("content-type", "application/json")
                .header("host", "chat.example")
                .body(Body::from(format!(
                    r#"{{"token":"{token}","username":"Carol","password":"password-carol","display_name":"卡罗"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let me = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("cookie", cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let body = me.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["username"], "Carol");
    let friends = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/friends")
                .header("cookie", cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(friends.status(), StatusCode::OK);

    let cookie_header = cookie.split(';').next().unwrap();
    let logged_out = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/logout")
                .header("cookie", cookie_header)
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logged_out.status(), StatusCode::NO_CONTENT);
    let after_logout = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("cookie", cookie_header)
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_logout.status(), StatusCode::UNAUTHORIZED);

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/login")
                .header("content-type", "application/json")
                .header("host", "chat.example")
                .body(Body::from(
                    r#"{"username":"Carol","password":"password-carol"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let login_cookie = login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let me_again = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("cookie", login_cookie.split(';').next().unwrap())
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me_again.status(), StatusCode::OK);
}

#[tokio::test(flavor = "current_thread")]
async fn spa_guest_routes_serve_login_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut opened = App::open(dir.path(), "correct-passphrase", "https://chat.example").unwrap();
    opened.insecure_cookies = true;
    let app = Arc::new(opened);
    let router =
        crate::http::wrap(Arc::clone(&app), crate::http::router()).with_state(Arc::clone(&app));
    for path in ["/", "/login", "/index.html"] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "chat.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert!(content_type.contains("text/html"), "{path} {content_type}");
        let cache = response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert!(cache.contains("no-cache"), "{path} {cache}");
        let body = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("登录"), "{path} 应包含登录文案");
        assert!(body.contains("password"), "{path} 应包含密码框");
        assert!(body.contains("/entry.js"), "{path} 应带上静态登录脚本");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn hierarchical_invites_quota_and_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let mut opened = App::open(dir.path(), "correct-passphrase", "https://chat.example").unwrap();
    opened.insecure_cookies = true;
    let app = Arc::new(opened);
    let router =
        crate::http::wrap(Arc::clone(&app), crate::http::router()).with_state(Arc::clone(&app));

    let seed = make_invite(&app);
    let (alice, _) = app
        .register(
            &seed,
            "Alice",
            "password-alice",
            "爱丽丝",
            "127.0.0.1",
            "test",
        )
        .await
        .unwrap();

    let bob_token = app
        .with_db(|conn| {
            let (_id, token, _) = crate::repo::create_invite(conn, alice.id, 7)?;
            Ok::<_, crate::error::AppErr>(token)
        })
        .unwrap();
    let (bob, _) = app
        .register(
            &bob_token,
            "Bob",
            "password-bob-ok",
            "鲍勃",
            "127.0.0.1",
            "test",
        )
        .await
        .unwrap();
    let carol_token = app
        .with_db(|conn| {
            let (_id, token, _) = crate::repo::create_invite(conn, bob.id, 7)?;
            Ok::<_, crate::error::AppErr>(token)
        })
        .unwrap();
    let (carol, _) = app
        .register(
            &carol_token,
            "Carol",
            "password-carol",
            "卡罗",
            "127.0.0.1",
            "test",
        )
        .await
        .unwrap();

    let invited_by = |user_id: i64| {
        app.with_db(|conn| {
            conn.query_row(
                "SELECT invited_by FROM users WHERE id=?1",
                [user_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(crate::error::AppErr::from)
        })
        .unwrap()
    };
    assert_eq!(invited_by(bob.id), Some(alice.id));
    assert_eq!(invited_by(carol.id), Some(bob.id));
    let alice_invitees = app
        .with_db(|conn| crate::repo::list_invitees(conn, alice.id))
        .unwrap();
    assert_eq!(alice_invitees.len(), 1);
    assert_eq!(alice_invitees[0].username, "Bob");
    let bob_invitees = app
        .with_db(|conn| crate::repo::list_invitees(conn, bob.id))
        .unwrap();
    assert_eq!(bob_invitees.len(), 1);
    assert_eq!(bob_invitees[0].username, "Carol");
    let carol_quota = app
        .with_db(|conn| crate::repo::invite_quota(conn, carol.id))
        .unwrap();
    assert_eq!(carol_quota.limit, 5);
    assert_eq!(carol_quota.used, 0);
    assert_eq!(carol_quota.remaining, 5);

    let pending_ids: Vec<i64> = (0..4)
        .map(|_| {
            app.with_db(|conn| {
                let (id, _, _) = crate::repo::create_invite(conn, alice.id, 7)?;
                Ok::<_, crate::error::AppErr>(id)
            })
            .unwrap()
        })
        .collect();
    let quota = app
        .with_db(|conn| crate::repo::invite_quota(conn, alice.id))
        .unwrap();
    assert_eq!(quota.used, 1);
    assert_eq!(quota.reserved, 4);
    assert_eq!(quota.remaining, 0);
    let sixth = app.with_db(|conn| crate::repo::create_invite(conn, alice.id, 7));
    match sixth {
        Err(crate::error::AppErr::Conflict(message)) => {
            assert_eq!(message, "邀请名额已用完，每人最多成功邀请 5 人");
        }
        other => panic!("第六个邀请应被拒绝，实际是 {other:?}"),
    }

    app.with_db(|conn| crate::repo::revoke_invite(conn, pending_ids[0]))
        .unwrap();
    let after_revoke = app
        .with_db(|conn| crate::repo::invite_quota(conn, alice.id))
        .unwrap();
    assert_eq!(after_revoke.reserved, 3);
    assert_eq!(after_revoke.remaining, 1);
    app.with_db(|conn| crate::repo::create_invite(conn, alice.id, 7))
        .unwrap();

    let expire_id = pending_ids[1];
    app.with_db(|conn| {
        conn.execute("UPDATE invites SET expires_at=1 WHERE id=?1", [expire_id])?;
        Ok::<_, crate::error::AppErr>(())
    })
    .unwrap();
    let after_expire = app
        .with_db(|conn| crate::repo::invite_quota(conn, alice.id))
        .unwrap();
    assert_eq!(after_expire.remaining, 1);
    app.with_db(|conn| crate::repo::create_invite(conn, alice.id, 7))
        .unwrap();
    app.with_db(|conn| {
        conn.execute(
            "UPDATE invites SET revoked=1 WHERE created_by=?1 AND used_by IS NULL AND revoked=0",
            [alice.id],
        )?;
        Ok::<_, crate::error::AppErr>(())
    })
    .unwrap();

    for n in 2..6 {
        let token = app
            .with_db(|conn| {
                let (_, token, _) = crate::repo::create_invite(conn, alice.id, 7)?;
                Ok::<_, crate::error::AppErr>(token)
            })
            .unwrap();
        app.register(
            &token,
            &format!("User{n}"),
            "password-user-ok",
            &format!("用户{n}"),
            "127.0.0.1",
            "test",
        )
        .await
        .unwrap();
    }
    let full = app
        .with_db(|conn| crate::repo::invite_quota(conn, alice.id))
        .unwrap();
    assert_eq!(full.used, 5);
    assert_eq!(full.remaining, 0);
    match app.with_db(|conn| crate::repo::create_invite(conn, alice.id, 7)) {
        Err(crate::error::AppErr::Conflict(message)) => {
            assert_eq!(message, "邀请名额已用完，每人最多成功邀请 5 人");
        }
        other => panic!("第 6 次成功邀请应被拒绝，实际是 {other:?}"),
    }

    app.with_db(|conn| {
        for n in 0..6 {
            conn.execute(
                "INSERT INTO invites (token_hash, created_by, created_at, expires_at, revoked, used_by, used_at)
                 VALUES (?1, ?2, 10, 9999999999, 0, ?3, 10)",
                rusqlite::params![unique_hash(n), bob.id, 10_000 + i64::from(n)],
            )?;
        }
        Ok::<_, crate::error::AppErr>(())
    })
    .unwrap();
    let over = app
        .with_db(|conn| crate::repo::invite_quota(conn, bob.id))
        .unwrap();
    assert!(over.used > 5, "历史超额应保留，实际 used={}", over.used);
    assert_eq!(over.remaining, 0);
    assert!(app
        .with_db(|conn| crate::repo::create_invite(conn, bob.id, 7))
        .is_err());

    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/login")
                .header("content-type", "application/json")
                .header("host", "chat.example")
                .body(Body::from(
                    r#"{"username":"Carol","password":"password-carol"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/invites")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .header("host", "chat.example")
                .body(Body::from(r#"{"days":7}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = created.into_body().collect().await.unwrap().to_bytes();
    let created_json: serde_json::Value = serde_json::from_slice(&created_body).unwrap();
    assert!(created_json["url"]
        .as_str()
        .unwrap()
        .contains("/#/register/"));
    assert_eq!(created_json["remaining"], 4);

    let listed = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/invites")
                .header("cookie", &cookie)
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_body = listed.into_body().collect().await.unwrap().to_bytes();
    let listed_json: serde_json::Value = serde_json::from_slice(&listed_body).unwrap();
    assert_eq!(listed_json["limit"], 5);
    assert_eq!(listed_json["reserved"], 1);

    let alice_login = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/login")
                .header("content-type", "application/json")
                .header("host", "chat.example")
                .body(Body::from(
                    r#"{"username":"Alice","password":"password-alice"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let alice_cookie = alice_login
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let blocked = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/invites")
                .header("content-type", "application/json")
                .header("cookie", &alice_cookie)
                .header("host", "chat.example")
                .body(Body::from(r#"{"days":7}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    let blocked_body = blocked.into_body().collect().await.unwrap().to_bytes();
    let blocked_json: serde_json::Value = serde_json::from_slice(&blocked_body).unwrap();
    assert_eq!(
        blocked_json["error"],
        "邀请名额已用完，每人最多成功邀请 5 人"
    );

    let anon = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/invites")
                .header("content-type", "application/json")
                .header("host", "chat.example")
                .body(Body::from(r#"{"days":7}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "current_thread")]
async fn legitimate_login_invite_and_refresh_stay_under_limits() {
    let (_dir, app, router) = http_app();
    let token = make_invite(&app);
    let (status, _, cookie) = send(
        &router,
        json_post(
            "/api/register",
            &format!(
                r#"{{"token":"{token}","username":"Dana","password":"password-dana","display_name":"达娜"}}"#
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("注册应设置 cookie");
    for _ in 0..6 {
        let (status, _, _) = send(
            &router,
            Request::builder()
                .uri("/api/me")
                .header("cookie", &cookie)
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = send(
            &router,
            Request::builder()
                .uri("/api/invites")
                .header("cookie", &cookie)
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = send(
            &router,
            Request::builder()
                .uri("/api/conversations")
                .header("cookie", &cookie)
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, created, _) = send(
        &router,
        authed_json_post("/api/invites", &cookie, r#"{"days":7}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["remaining"], 4);
    let (status, created, _) = send(
        &router,
        authed_json_post("/api/invites", &cookie, r#"{"days":7}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["remaining"], 3);

    let (logout_status, _, _) = send(
        &router,
        Request::builder()
            .method("POST")
            .uri("/api/logout")
            .header("cookie", &cookie)
            .header("host", "chat.example")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(logout_status, StatusCode::NO_CONTENT);
    let (status, _, login_cookie) = send(
        &router,
        json_post(
            "/api/login",
            r#"{"username":"Dana","password":"password-dana"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(login_cookie.is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn login_brute_force_locks_user_but_two_mistakes_still_succeed() {
    let (_dir, app, router) = http_app();
    let token = make_invite(&app);
    app.register(
        &token,
        "Eve",
        "password-eve-ok",
        "伊芙",
        "127.0.0.1",
        "test",
    )
    .await
    .unwrap();

    for n in 0..crate::rate::LOGIN_USER {
        let (status, body, _) = send(
            &router,
            json_post(
                "/api/login",
                &format!(r#"{{"username":"Eve","password":"wrong-password-{n}"}}"#),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "第 {} 次失败登录", n + 1);
        assert_eq!(body["error"], "用户名或密码错误");
    }
    let (status, body, _) = send(
        &router,
        json_post(
            "/api/login",
            r#"{"username":"Eve","password":"password-eve-ok"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "操作太频繁，请稍后再试");

    let (_dir, app, router) = http_app();
    let token = make_invite(&app);
    app.register(
        &token,
        "Eve",
        "password-eve-ok",
        "伊芙",
        "127.0.0.1",
        "test",
    )
    .await
    .unwrap();
    for _ in 0..2 {
        let (status, _, _) = send(
            &router,
            json_post(
                "/api/login",
                r#"{"username":"Eve","password":"wrong-password"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, cookie) = send(
        &router,
        json_post(
            "/api/login",
            r#"{"username":"Eve","password":"password-eve-ok"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn register_spam_and_invite_guessing_are_limited() {
    let (_dir, app, router) = http_app();
    let valid = make_invite(&app);
    let (status, _, cookie) = send(
        &router,
        json_post(
            "/api/register",
            &format!(
                r#"{{"token":"{valid}","username":"Fay","password":"password-fay-ok","display_name":"费伊"}}"#
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_some());

    let (_dir, _app, router) = http_app();
    let fake = "A".repeat(32);
    for n in 0..crate::rate::REGISTER_IP {
        let (status, body, _) = send(
            &router,
            json_post(
                "/api/register",
                &format!(
                    r#"{{"token":"{fake}","username":"User{n}","password":"password-user-ok","display_name":"用户{n}"}}"#
                ),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "第 {} 次无效邀请", n + 1);
        assert_eq!(body["error"], "邀请无效");
    }
    let (status, body, _) = send(
        &router,
        json_post(
            "/api/register",
            &format!(
                r#"{{"token":"{fake}","username":"Last","password":"password-last-ok","display_name":"最后"}}"#
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "操作太频繁，请稍后再试");
}

#[tokio::test(flavor = "current_thread")]
async fn invite_create_flood_is_limited_after_quota_is_freed() {
    let (_dir, app, router) = http_app();
    let token = make_invite(&app);
    let (status, _, cookie) = send(
        &router,
        json_post(
            "/api/register",
            &format!(
                r#"{{"token":"{token}","username":"Gus","password":"password-gus-ok","display_name":"格斯"}}"#
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.unwrap();

    let mut ids = Vec::new();
    for n in 0..crate::rate::INVITE_CREATE {
        if ids.len() == 5 {
            for id in ids.drain(..) {
                let (status, _, _) = send(
                    &router,
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/invites/{id}/revoke"))
                        .header("cookie", &cookie)
                        .header("host", "chat.example")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
                assert_eq!(status, StatusCode::NO_CONTENT, "作废邀请 {id}");
            }
        }
        let (status, body, _) = send(
            &router,
            authed_json_post("/api/invites", &cookie, r#"{"days":7}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "第 {} 次生成邀请", n + 1);
        ids.push(body["id"].as_i64().unwrap());
    }
    let (status, body, _) = send(
        &router,
        authed_json_post("/api/invites", &cookie, r#"{"days":7}"#),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "操作太频繁，请稍后再试");
}

#[tokio::test(flavor = "current_thread")]
async fn unauthenticated_api_flood_is_limited_without_blocking_health() {
    let (_dir, _app, router) = http_app();
    for n in 0..crate::rate::API_IP {
        let (status, _, _) = send(
            &router,
            Request::builder()
                .uri("/api/me")
                .header("host", "chat.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "第 {} 次匿名请求", n + 1);
    }
    let (status, body, _) = send(
        &router,
        Request::builder()
            .uri("/api/me")
            .header("host", "chat.example")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "操作太频繁，请稍后再试");
    let (status, body, _) = send(
        &router,
        Request::builder()
            .uri("/api/health")
            .header("host", "chat.example")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_json_is_rejected_while_normal_login_still_works() {
    let (_dir, app, router) = http_app();
    let token = make_invite(&app);
    app.register(
        &token,
        "Ivy",
        "password-ivy-ok",
        "艾薇",
        "127.0.0.1",
        "test",
    )
    .await
    .unwrap();
    let huge = format!(
        r#"{{"username":"Ivy","password":"{}"}}"#,
        "p".repeat(70_000)
    );
    let (status, body, _) = send(
        &router,
        Request::builder()
            .method("POST")
            .uri("/api/login")
            .header("content-type", "application/json")
            .header("content-length", huge.len().to_string())
            .header("host", "chat.example")
            .body(Body::from(huge))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "请求过大");
    let (status, _, cookie) = send(
        &router,
        json_post(
            "/api/login",
            r#"{"username":"Ivy","password":"password-ivy-ok"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_some());
}

fn http_app() -> (tempfile::TempDir, Arc<App>, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let mut opened = App::open(dir.path(), "correct-passphrase", "https://chat.example").unwrap();
    opened.insecure_cookies = true;
    let app = Arc::new(opened);
    let router =
        crate::http::wrap(Arc::clone(&app), crate::http::router()).with_state(Arc::clone(&app));
    (dir, app, router)
}

fn json_post(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("host", "chat.example")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn authed_json_post(uri: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .header("host", "chat.example")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn send(
    router: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json, cookie)
}

fn unique_hash(n: u8) -> Vec<u8> {
    let mut hash = vec![0u8; 32];
    hash[0] = n;
    hash[1] = 0xff;
    hash
}

fn make_invite(app: &App) -> String {
    app.with_db(|conn| {
        let (_id, token, _exp) = crate::repo::create_invite(conn, 0, 7)?;
        Ok::<_, crate::error::AppErr>(token)
    })
    .unwrap()
}

fn tiny_jpeg() -> Vec<u8> {
    let image = image::RgbImage::from_fn(80, 40, |x, y| {
        image::Rgb([(x * 3) as u8, (y * 5) as u8, 40])
    });
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    encoder
        .encode(image.as_raw(), 80, 40, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}
