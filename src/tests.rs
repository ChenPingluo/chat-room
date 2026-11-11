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
        assert_ne!(&disk[..2], &[0xff, 0xd8], "磁盘上不应保存未加密的 JPEG");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn http_login_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut opened = App::open(dir.path(), "correct-passphrase", "https://chat.example").unwrap();
    opened.insecure_cookies = true;
    let app = Arc::new(opened);
    let token = make_invite(&app);
    let router = crate::http::wrap(crate::http::router()).with_state(Arc::clone(&app));
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
