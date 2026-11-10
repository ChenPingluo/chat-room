use sqlx::{FromRow, SqlitePool};
use tokio::sync::Semaphore;

use crate::auth::AuthUser;
use crate::dto::{InviteCreated, InviteDto, MeDto, PublicUser, SessionDto};
use crate::error::AppError;
use crate::password::{self, validate_password};
use crate::state::AppState;
use crate::util::{self, now_ms, SESSION_TTL_MS};

#[derive(FromRow)]
struct UserRow {
    id: i64,
    uid: String,
    username: String,
    password_hash: String,
    is_admin: i64,
    disabled: i64,
    created_at: i64,
}

pub fn me_of(user: &AuthUser, created_at: i64) -> MeDto {
    MeDto {
        id: user.id,
        uid: user.uid.clone(),
        username: user.username.clone(),
        is_admin: user.is_admin,
        created_at,
    }
}

pub async fn load_me(state: &AppState, user: &AuthUser) -> Result<MeDto, AppError> {
    let created_at: i64 = sqlx::query_scalar("SELECT created_at FROM users WHERE id = ?")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
    Ok(me_of(user, created_at))
}

pub async fn hash_async(sem: &Semaphore, password: String) -> Result<String, AppError> {
    let _permit = sem
        .acquire()
        .await
        .map_err(|_| AppError::internal(anyhow::anyhow!("密码计算队列已关闭")))?;
    tokio::task::spawn_blocking(move || password::hash_password(&password))
        .await
        .map_err(AppError::internal)?
}

pub async fn verify_async(
    sem: &Semaphore,
    password: String,
    encoded: String,
) -> Result<bool, AppError> {
    let _permit = sem
        .acquire()
        .await
        .map_err(|_| AppError::internal(anyhow::anyhow!("密码计算队列已关闭")))?;
    tokio::task::spawn_blocking(move || password::verify_password(&password, &encoded))
        .await
        .map_err(AppError::internal)
}

pub async fn provision_admin(
    state: &AppState,
    username: &str,
    password: &str,
) -> Result<i64, AppError> {
    create_user(state, username, password, true).await
}

async fn create_user(
    state: &AppState,
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<i64, AppError> {
    util::validate_username(username)?;
    validate_password(password, username)?;
    let password_hash = hash_async(&state.argon, password.to_string()).await?;
    insert_user(&state.db, username, &password_hash, is_admin).await
}

async fn insert_user(
    db: &SqlitePool,
    username: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<i64, AppError> {
    let now = now_ms();
    let lower = username.to_lowercase();
    for _ in 0..8 {
        let uid = util::random_uid()?;
        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (uid, username, username_lower, password_hash, is_admin, disabled, created_at)
             VALUES (?, ?, ?, ?, ?, 0, ?) RETURNING id",
        )
        .bind(&uid)
        .bind(username)
        .bind(&lower)
        .bind(password_hash)
        .bind(i64::from(is_admin))
        .bind(now)
        .fetch_one(db)
        .await;
        match result {
            Ok(id) => return Ok(id),
            Err(sqlx::Error::Database(err)) => {
                let message = err.message();
                if message.contains("username_lower") {
                    return Err(AppError::conflict("用户名已被占用"));
                }
                if message.contains("uid") {
                    continue;
                }
                return Err(sqlx::Error::Database(err).into());
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(AppError::internal(anyhow::anyhow!("无法分配 UID")))
}

pub async fn register(
    state: &AppState,
    token: &str,
    username: &str,
    password: &str,
    user_agent: &str,
) -> Result<(MeDto, String), AppError> {
    util::validate_username(username)?;
    validate_password(password, username)?;
    let token_hash = util::token_hash(token);
    let invite_ok: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM invites WHERE token_hash = ? AND revoked = 0 AND used_by IS NULL AND expires_at > ?",
    )
    .bind(&token_hash)
    .bind(now_ms())
    .fetch_optional(&state.db)
    .await?;
    if invite_ok.is_none() {
        return Err(AppError::bad("邀请链接无效或已过期"));
    }
    let password_hash = hash_async(&state.argon, password.to_string()).await?;
    let mut tx = state.db.begin().await?;
    let user_id = insert_user_tx(&mut tx, username, &password_hash, false).await?;
    let now = now_ms();
    let claimed = sqlx::query(
        "UPDATE invites SET used_by = ?, used_at = ? WHERE token_hash = ? AND revoked = 0 AND used_by IS NULL AND expires_at > ?",
    )
    .bind(user_id)
    .bind(now)
    .bind(&token_hash)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if claimed.rows_affected() != 1 {
        return Err(AppError::bad("邀请链接无效或已过期"));
    }
    let created_at: i64 = sqlx::query_scalar("SELECT created_at FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    let uid: String = sqlx::query_scalar("SELECT uid FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    let session = insert_session(&mut tx, user_id, user_agent, None).await?;
    tx.commit().await?;
    let me = MeDto {
        id: user_id,
        uid,
        username: username.to_string(),
        is_admin: false,
        created_at,
    };
    Ok((me, session))
}

async fn insert_user_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<i64, AppError> {
    let now = now_ms();
    let lower = username.to_lowercase();
    for _ in 0..8 {
        let uid = util::random_uid()?;
        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (uid, username, username_lower, password_hash, is_admin, disabled, created_at)
             VALUES (?, ?, ?, ?, ?, 0, ?) RETURNING id",
        )
        .bind(&uid)
        .bind(username)
        .bind(&lower)
        .bind(password_hash)
        .bind(i64::from(is_admin))
        .bind(now)
        .fetch_one(&mut **tx)
        .await;
        match result {
            Ok(id) => return Ok(id),
            Err(sqlx::Error::Database(err)) => {
                let message = err.message();
                if message.contains("username_lower") {
                    return Err(AppError::conflict("用户名已被占用"));
                }
                if message.contains("uid") {
                    continue;
                }
                return Err(sqlx::Error::Database(err).into());
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(AppError::internal(anyhow::anyhow!("无法分配 UID")))
}

async fn insert_session(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: i64,
    user_agent: &str,
    keep_id: Option<i64>,
) -> Result<String, AppError> {
    let now = now_ms();
    let token = util::random_token()?;
    let hash = util::token_hash(&token);
    let agent = util::truncate_chars(user_agent, 200);
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, last_seen, expires_at, user_agent) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&hash)
    .bind(user_id)
    .bind(now)
    .bind(now)
    .bind(now + SESSION_TTL_MS)
    .bind(&agent)
    .execute(&mut **tx)
    .await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM sessions WHERE user_id = ? ORDER BY created_at DESC, id DESC",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    for id in ids.into_iter().skip(10) {
        if keep_id == Some(id) {
            continue;
        }
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(token)
}

fn lock_ms(failures: i64) -> i64 {
    match failures {
        0..=4 => 0,
        5..=7 => 60_000,
        8..=10 => 300_000,
        11..=13 => 900_000,
        _ => 3_600_000,
    }
}

async fn is_locked(db: &SqlitePool, key: &str, now: i64) -> Result<bool, AppError> {
    let until: Option<i64> =
        sqlx::query_scalar("SELECT locked_until FROM login_attempts WHERE key = ?")
            .bind(key)
            .fetch_optional(db)
            .await?;
    Ok(until.unwrap_or(0) > now)
}

async fn bump_failure(db: &SqlitePool, key: &str, now: i64) -> Result<(), AppError> {
    let failures: i64 = sqlx::query_scalar(
        "INSERT INTO login_attempts (key, failures, locked_until, updated_at) VALUES (?, 1, 0, ?)
         ON CONFLICT(key) DO UPDATE SET failures = failures + 1, updated_at = excluded.updated_at
         RETURNING failures",
    )
    .bind(key)
    .bind(now)
    .fetch_one(db)
    .await?;
    let lock = lock_ms(failures);
    let until = if lock == 0 { 0 } else { now + lock };
    sqlx::query("UPDATE login_attempts SET locked_until = ? WHERE key = ?")
        .bind(until)
        .bind(key)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn login(
    state: &AppState,
    username: &str,
    password: &str,
    ip: &str,
    user_agent: &str,
) -> Result<(MeDto, String), AppError> {
    let lower = username.trim().to_lowercase();
    if lower.is_empty() || lower.len() > 20 {
        return Err(AppError::LoginFailed);
    }
    let ip_key = format!("ip:{}", util::truncate_chars(ip, 64));
    let name_key = format!("name:{lower}");
    let now = now_ms();
    if is_locked(&state.db, &ip_key, now).await? || is_locked(&state.db, &name_key, now).await? {
        return Err(AppError::too_many("尝试次数过多，请稍后再试"));
    }
    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, uid, username, password_hash, is_admin, disabled, created_at FROM users WHERE username_lower = ?",
    )
    .bind(&lower)
    .fetch_optional(&state.db)
    .await?;
    let hash = match &user {
        Some(user) if user.disabled == 0 => user.password_hash.clone(),
        _ => state.dummy_hash.as_ref().clone(),
    };
    let matches = verify_async(&state.argon, password.to_string(), hash).await?;
    let user = user.filter(|user| user.disabled == 0 && matches);
    let Some(user) = user else {
        bump_failure(&state.db, &ip_key, now).await?;
        bump_failure(&state.db, &name_key, now).await?;
        tracing::warn!(ip = %ip_key, "登录失败");
        return Err(AppError::LoginFailed);
    };
    sqlx::query("DELETE FROM login_attempts WHERE key IN (?, ?)")
        .bind(&ip_key)
        .bind(&name_key)
        .execute(&state.db)
        .await?;
    let mut tx = state.db.begin().await?;
    let token = insert_session(&mut tx, user.id, user_agent, None).await?;
    tx.commit().await?;
    tracing::info!(user_id = user.id, "登录成功");
    Ok((
        MeDto {
            id: user.id,
            uid: user.uid,
            username: user.username,
            is_admin: user.is_admin != 0,
            created_at: user.created_at,
        },
        token,
    ))
}

pub async fn logout(state: &AppState, user: &AuthUser) -> Result<(), AppError> {
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(user.session_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn logout_all(state: &AppState, user: &AuthUser) -> Result<(), AppError> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn change_password(
    state: &AppState,
    user: &AuthUser,
    current: &str,
    next: &str,
) -> Result<(), AppError> {
    validate_password(next, &user.username)?;
    let encoded: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;
    if !verify_async(&state.argon, current.to_string(), encoded).await? {
        return Err(AppError::bad("当前密码不正确"));
    }
    let new_hash = hash_async(&state.argon, next.to_string()).await?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND id != ?")
        .bind(user.id)
        .bind(user.session_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn list_sessions(state: &AppState, user: &AuthUser) -> Result<Vec<SessionDto>, AppError> {
    #[derive(FromRow)]
    struct Row {
        id: i64,
        created_at: i64,
        last_seen: i64,
        expires_at: i64,
        user_agent: String,
    }
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, created_at, last_seen, expires_at, user_agent FROM sessions WHERE user_id = ? AND expires_at > ? ORDER BY last_seen DESC",
    )
    .bind(user.id)
    .bind(now_ms())
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SessionDto {
            current: row.id == user.session_id,
            id: row.id,
            created_at: row.created_at,
            last_seen: row.last_seen,
            expires_at: row.expires_at,
            user_agent: row.user_agent,
        })
        .collect())
}

pub async fn revoke_session(
    state: &AppState,
    user: &AuthUser,
    session_id: i64,
) -> Result<(), AppError> {
    let changed = sqlx::query("DELETE FROM sessions WHERE id = ? AND user_id = ?")
        .bind(session_id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub async fn preview_invite(state: &AppState, token: &str) -> Result<i64, AppError> {
    let expires_at: Option<i64> = sqlx::query_scalar(
        "SELECT expires_at FROM invites WHERE token_hash = ? AND revoked = 0 AND used_by IS NULL AND expires_at > ?",
    )
    .bind(util::token_hash(token))
    .bind(now_ms())
    .fetch_optional(&state.db)
    .await?;
    expires_at.ok_or_else(|| AppError::bad("邀请链接无效或已过期"))
}

pub async fn create_invite(
    state: &AppState,
    admin: &AuthUser,
    days: i64,
) -> Result<InviteCreated, AppError> {
    if !admin.is_admin {
        return Err(AppError::forbidden("只有管理员可以创建邀请"));
    }
    if !(1..=30).contains(&days) {
        return Err(AppError::bad("邀请有效期需要在 1 到 30 天之间"));
    }
    let token = util::random_token()?;
    let now = now_ms();
    let expires_at = now + days * 24 * 60 * 60 * 1000;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO invites (token_hash, created_by, expires_at, revoked, created_at) VALUES (?, ?, ?, 0, ?) RETURNING id",
    )
    .bind(util::token_hash(&token))
    .bind(admin.id)
    .bind(expires_at)
    .bind(now)
    .fetch_one(&state.db)
    .await?;
    let url = format!("{}/invite#{}", state.config.public_base_url, token);
    Ok(InviteCreated {
        invite: InviteDto {
            id,
            created_at: now,
            expires_at,
            revoked: false,
            used_at: None,
            used_by: None,
        },
        url,
        token,
    })
}

pub async fn create_invite_for(
    state: &AppState,
    admin_id: i64,
    days: i64,
) -> Result<InviteCreated, AppError> {
    let admin = AuthUser {
        id: admin_id,
        uid: String::new(),
        username: String::new(),
        is_admin: true,
        session_id: 0,
    };
    create_invite(state, &admin, days).await
}

pub async fn list_invites(state: &AppState, admin: &AuthUser) -> Result<Vec<InviteDto>, AppError> {
    if !admin.is_admin {
        return Err(AppError::forbidden("只有管理员可以查看邀请"));
    }
    #[derive(FromRow)]
    struct Row {
        id: i64,
        created_at: i64,
        expires_at: i64,
        revoked: i64,
        used_at: Option<i64>,
        used_id: Option<i64>,
        used_uid: Option<String>,
        used_username: Option<String>,
    }
    let rows = sqlx::query_as::<_, Row>(
        "SELECT i.id, i.created_at, i.expires_at, i.revoked, i.used_at,
                u.id AS used_id, u.uid AS used_uid, u.username AS used_username
         FROM invites i LEFT JOIN users u ON u.id = i.used_by
         ORDER BY i.id DESC LIMIT 100",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| InviteDto {
            id: row.id,
            created_at: row.created_at,
            expires_at: row.expires_at,
            revoked: row.revoked != 0,
            used_at: row.used_at,
            used_by: match (row.used_id, row.used_uid, row.used_username) {
                (Some(id), Some(uid), Some(username)) => Some(PublicUser {
                    id,
                    uid,
                    username,
                    relation: None,
                }),
                _ => None,
            },
        })
        .collect())
}

pub async fn revoke_invite(
    state: &AppState,
    admin: &AuthUser,
    invite_id: i64,
) -> Result<(), AppError> {
    if !admin.is_admin {
        return Err(AppError::forbidden("只有管理员可以作废邀请"));
    }
    let changed = sqlx::query("UPDATE invites SET revoked = 1 WHERE id = ? AND used_by IS NULL")
        .bind(invite_id)
        .execute(&state.db)
        .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}
