use std::collections::HashSet;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;

use crate::crypto::{self, Key};
use crate::error::AppErr;
use crate::models::{ConversationOut, MemberOut, MessageOut, PublicUser, User};
use crate::util::{
    self, b64, now, preview, random_bytes, sha256, GROUP_LIMIT, RET_MONTH, SESSION_SECS,
};

const USER_COLS: &str =
    "id, uid, username, display_name, is_admin, disabled, totp_enabled, created_at";
const USER_COLS_U: &str =
    "u.id, u.uid, u.username, u.display_name, u.is_admin, u.disabled, u.totp_enabled, u.created_at";

fn map_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        uid: row.get(1)?,
        username: row.get(2)?,
        display_name: row.get(3)?,
        is_admin: row.get::<_, i64>(4)? != 0,
        disabled: row.get::<_, i64>(5)? != 0,
        totp_enabled: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

pub struct LoginRecord {
    pub user: User,
    pub password_hash: String,
    pub totp_secret: Option<Vec<u8>>,
}

pub struct Posted {
    pub message: MessageOut,
    pub members: Vec<i64>,
}

pub struct InviteRow {
    pub id: i64,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked: bool,
    pub creator: Option<String>,
    pub used_by: Option<String>,
    pub used_at: Option<i64>,
}

pub struct SessionRow {
    pub id: i64,
    pub created_at: i64,
    pub last_seen: i64,
    pub ip: String,
    pub user_agent: String,
    pub current: bool,
}

#[derive(Serialize)]
pub struct FriendRequestRow {
    pub id: i64,
    pub incoming: bool,
    pub user: PublicUser,
    pub created_at: i64,
}

pub struct ConvStat {
    pub id: i64,
    pub kind: String,
    pub retention: i64,
    pub created_at: i64,
    pub members: i64,
    pub messages: i64,
    pub image_bytes: i64,
}

fn aad(conversation: i64, sender: i64, created: i64, kind: &str) -> String {
    format!("msg:{conversation}:{sender}:{created}:{kind}")
}

fn name_aad(conversation: i64) -> String {
    format!("conv-name:{conversation}")
}

pub fn user_count(conn: &Connection) -> Result<i64, AppErr> {
    Ok(conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?)
}

pub fn setting(conn: &Connection, key: &str) -> Result<Option<Vec<u8>>, AppErr> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &[u8]) -> Result<(), AppErr> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn audit(
    conn: &Connection,
    actor: Option<i64>,
    action: &str,
    detail: &str,
    at: i64,
) -> Result<(), AppErr> {
    let detail = preview(detail, 180);
    conn.execute(
        "INSERT INTO audit_log (actor_id, action, detail, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![actor, action, detail, at],
    )?;
    Ok(())
}

fn alloc_uid(conn: &Connection) -> Result<String, AppErr> {
    for _ in 0..40 {
        let value = u32::from_le_bytes(random_bytes::<4>()) % 9_000_000 + 1_000_000;
        let uid = value.to_string();
        let exists: i64 =
            conn.query_row("SELECT COUNT(*) FROM users WHERE uid=?1", [&uid], |row| {
                row.get(0)
            })?;
        if exists == 0 {
            return Ok(uid);
        }
    }
    Err(AppErr::Internal)
}

#[allow(clippy::too_many_arguments)]
pub fn insert_user(
    conn: &Connection,
    key: &Key,
    username: &str,
    password_hash: &str,
    display_name: &str,
    totp_secret: Option<&[u8]>,
    is_admin: bool,
    at: i64,
) -> Result<User, AppErr> {
    let username = util::clean_username(username).map_err(AppErr::BadRequest)?;
    let display_name = util::clean_display(display_name).map_err(AppErr::BadRequest)?;
    let lower = username.to_ascii_lowercase();
    let taken: i64 = conn.query_row(
        "SELECT COUNT(*) FROM users WHERE username_lower=?1",
        [&lower],
        |row| row.get(0),
    )?;
    if taken > 0 {
        return Err(AppErr::Conflict("用户名已存在"));
    }
    let uid = alloc_uid(conn)?;
    let secret = match totp_secret {
        Some(secret) => Some(crypto::seal(key, format!("totp:{uid}").as_bytes(), secret)?),
        None => None,
    };
    conn.execute(
        "INSERT INTO users (uid, username, username_lower, password_hash, display_name, totp_secret, totp_enabled, is_admin, disabled, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
        params![
            uid,
            username,
            lower,
            password_hash,
            display_name,
            secret,
            i64::from(totp_secret.is_some()),
            i64::from(is_admin),
            at
        ],
    )?;
    user_by_id(conn, conn.last_insert_rowid())
}

pub fn user_by_id(conn: &Connection, id: i64) -> Result<User, AppErr> {
    conn.query_row(
        &format!("SELECT {USER_COLS} FROM users WHERE id=?1"),
        [id],
        map_user,
    )
    .optional()?
    .ok_or(AppErr::NotFound)
}

pub fn login_record(conn: &Connection, username: &str) -> Result<Option<LoginRecord>, AppErr> {
    let lower = username.trim().to_ascii_lowercase();
    let row = conn
        .query_row(
            &format!(
                "SELECT {USER_COLS}, password_hash, totp_secret FROM users WHERE username_lower=?1"
            ),
            [&lower],
            |row| {
                Ok(LoginRecord {
                    user: map_user(row)?,
                    password_hash: row.get(8)?,
                    totp_secret: row.get(9)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn open_totp(key: &Key, user: &User, packed: &[u8]) -> Result<Vec<u8>, AppErr> {
    crypto::open(key, format!("totp:{}", user.uid).as_bytes(), packed)
}

pub fn create_session(
    conn: &Connection,
    user_id: i64,
    kind: &str,
    ip: &str,
    user_agent: &str,
    at: i64,
) -> Result<String, AppErr> {
    let raw = random_bytes::<32>();
    let token = b64(&raw);
    let hash = sha256(&raw);
    let agent = preview(user_agent, 180);
    let ip = preview(ip, 64);
    conn.execute(
        "INSERT INTO sessions (user_id, token_hash, kind, created_at, last_seen, expires_at, user_agent, ip) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7)",
        params![user_id, hash.to_vec(), kind, at, at + SESSION_SECS, agent, ip],
    )?;
    Ok(token)
}

pub fn session_user(
    conn: &Connection,
    token: &str,
    kind: &str,
    at: i64,
) -> Result<crate::models::SessionUser, AppErr> {
    let raw = util::b64_decode(token).map_err(|_| AppErr::Unauthorized("请先登录"))?;
    if raw.len() != 32 {
        return Err(AppErr::Unauthorized("请先登录"));
    }
    let hash = sha256(&raw);
    let found = conn
        .query_row(
            "SELECT s.id, s.expires_at, s.last_seen, u.id FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=?1 AND s.kind=?2",
            params![hash.to_vec(), kind],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
        )
        .optional()?;
    let Some((session_id, expires, last_seen, user_id)) = found else {
        return Err(AppErr::Unauthorized("请先登录"));
    };
    if expires <= at {
        conn.execute("DELETE FROM sessions WHERE id=?1", [session_id])?;
        return Err(AppErr::Unauthorized("登录已过期"));
    }
    let user = user_by_id(conn, user_id)?;
    if user.disabled {
        conn.execute("DELETE FROM sessions WHERE user_id=?1", [user.id])?;
        return Err(AppErr::Unauthorized("账号已停用"));
    }
    if kind == "admin" && (!user.is_admin || !user.totp_enabled) {
        return Err(AppErr::Forbidden("需要已开启两步验证的管理员"));
    }
    if last_seen + 60 < at {
        conn.execute(
            "UPDATE sessions SET last_seen=?1 WHERE id=?2",
            params![at, session_id],
        )?;
    }
    Ok(crate::models::SessionUser { session_id, user })
}

pub fn session_live(
    conn: &Connection,
    session_id: i64,
    user_id: i64,
    at: i64,
) -> Result<bool, AppErr> {
    let expires: Option<i64> = conn
        .query_row(
            "SELECT s.expires_at FROM sessions s
             JOIN users u ON u.id=s.user_id
             WHERE s.id=?1 AND s.user_id=?2 AND u.disabled=0",
            params![session_id, user_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(expires.is_some_and(|expires| expires > at))
}

pub fn list_sessions(
    conn: &Connection,
    user_id: i64,
    kind: &str,
    current: i64,
) -> Result<Vec<SessionRow>, AppErr> {
    let mut stmt = conn.prepare(
        "SELECT id, created_at, last_seen, ip, user_agent FROM sessions WHERE user_id=?1 AND kind=?2 ORDER BY id DESC",
    )?;
    let rows = stmt.query_map(params![user_id, kind], |row| {
        let id: i64 = row.get(0)?;
        Ok(SessionRow {
            id,
            created_at: row.get(1)?,
            last_seen: row.get(2)?,
            ip: row.get(3)?,
            user_agent: row.get(4)?,
            current: id == current,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn delete_session(conn: &Connection, user_id: i64, session_id: i64) -> Result<(), AppErr> {
    conn.execute(
        "DELETE FROM sessions WHERE id=?1 AND user_id=?2",
        params![session_id, user_id],
    )?;
    Ok(())
}

pub fn delete_other_sessions(conn: &Connection, user_id: i64, keep: i64) -> Result<(), AppErr> {
    conn.execute(
        "DELETE FROM sessions WHERE user_id=?1 AND id!=?2",
        params![user_id, keep],
    )?;
    Ok(())
}

pub fn delete_user_sessions(conn: &Connection, user_id: i64) -> Result<(), AppErr> {
    conn.execute("DELETE FROM sessions WHERE user_id=?1", [user_id])?;
    Ok(())
}

pub fn set_password(conn: &Connection, user_id: i64, password_hash: &str) -> Result<(), AppErr> {
    conn.execute(
        "UPDATE users SET password_hash=?1 WHERE id=?2",
        params![password_hash, user_id],
    )?;
    Ok(())
}

pub fn set_totp_pending(
    conn: &Connection,
    key: &Key,
    user: &User,
    secret: &[u8],
) -> Result<(), AppErr> {
    let packed = crypto::seal(key, format!("totp:{}", user.uid).as_bytes(), secret)?;
    conn.execute(
        "UPDATE users SET totp_pending=?1 WHERE id=?2",
        params![packed, user.id],
    )?;
    Ok(())
}

pub fn totp_pending(conn: &Connection, key: &Key, user: &User) -> Result<Option<Vec<u8>>, AppErr> {
    let packed: Option<Vec<u8>> = conn.query_row(
        "SELECT totp_pending FROM users WHERE id=?1",
        [user.id],
        |row| row.get(0),
    )?;
    match packed {
        Some(packed) => Ok(Some(open_totp(key, user, &packed)?)),
        None => Ok(None),
    }
}

pub fn enable_totp(conn: &Connection, user_id: i64) -> Result<(), AppErr> {
    conn.execute(
        "UPDATE users SET totp_secret=totp_pending, totp_pending=NULL, totp_enabled=1 WHERE id=?1 AND totp_pending IS NOT NULL",
        [user_id],
    )?;
    if conn.changes() != 1 {
        return Err(AppErr::BadRequest("请先开始设置两步验证"));
    }
    Ok(())
}

pub fn disable_totp(conn: &Connection, user_id: i64) -> Result<(), AppErr> {
    conn.execute(
        "UPDATE users SET totp_secret=NULL, totp_pending=NULL, totp_enabled=0 WHERE id=?1",
        [user_id],
    )?;
    Ok(())
}

pub fn register(
    conn: &mut Connection,
    key: &Key,
    token: &str,
    username: &str,
    password_hash: &str,
    display_name: &str,
) -> Result<User, AppErr> {
    let hash = sha256(token.as_bytes()).to_vec();
    let at = now();
    let tx = conn.transaction()?;
    let invite = tx
        .query_row(
            "SELECT id, expires_at, revoked, used_by FROM invites WHERE token_hash=?1",
            [&hash],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((id, expires, revoked, used)) = invite else {
        return Err(AppErr::BadRequest("邀请无效"));
    };
    if !invite_open(revoked, used, expires, at) {
        return Err(AppErr::BadRequest("邀请无效"));
    }
    let user = insert_user(
        &tx,
        key,
        username,
        password_hash,
        display_name,
        None,
        false,
        at,
    )?;
    tx.execute(
        "UPDATE invites SET used_by=?1, used_at=?2 WHERE id=?3 AND used_by IS NULL AND revoked=0",
        params![user.id, at, id],
    )?;
    if tx.changes() != 1 {
        return Err(AppErr::BadRequest("邀请无效"));
    }
    tx.commit()?;
    Ok(user)
}

pub fn create_invite(
    conn: &Connection,
    admin_id: i64,
    days: i64,
) -> Result<(i64, String, i64), AppErr> {
    if !(1..=30).contains(&days) {
        return Err(AppErr::BadRequest("有效期需要在 1 到 30 天之间"));
    }
    let token = b64(&random_bytes::<32>());
    let at = now();
    let expires = at + days * 86_400;
    conn.execute(
        "INSERT INTO invites (token_hash, created_by, created_at, expires_at, revoked) VALUES (?1, ?2, ?3, ?4, 0)",
        params![sha256(token.as_bytes()).to_vec(), admin_id, at, expires],
    )?;
    Ok((conn.last_insert_rowid(), token, expires))
}

pub fn revoke_invite(conn: &Connection, id: i64) -> Result<(), AppErr> {
    conn.execute("UPDATE invites SET revoked=1 WHERE id=?1", [id])?;
    if conn.changes() != 1 {
        return Err(AppErr::NotFound);
    }
    Ok(())
}

pub fn list_invites(conn: &Connection) -> Result<Vec<InviteRow>, AppErr> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.created_at, i.expires_at, i.revoked, c.username, u.username, i.used_at
         FROM invites i
         LEFT JOIN users c ON c.id=i.created_by
         LEFT JOIN users u ON u.id=i.used_by
         ORDER BY i.id DESC LIMIT 200",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(InviteRow {
            id: row.get(0)?,
            created_at: row.get(1)?,
            expires_at: row.get(2)?,
            revoked: row.get::<_, i64>(3)? != 0,
            creator: row.get(4)?,
            used_by: row.get(5)?,
            used_at: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn search_users(conn: &Connection, actor: i64, query: &str) -> Result<Vec<User>, AppErr> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 32 {
        return Err(AppErr::BadRequest("请输入完整的用户名或 UID"));
    }
    let lower = query.to_ascii_lowercase();
    let mut stmt = conn.prepare(&format!(
        "SELECT {USER_COLS} FROM users WHERE disabled=0 AND (username_lower=?1 OR uid=?2) LIMIT 2"
    ))?;
    let rows = stmt.query_map(params![lower, query], map_user)?;
    let mut users = Vec::new();
    for row in rows {
        let user = row?;
        if user.id != actor {
            users.push(user);
        }
    }
    Ok(users)
}

pub fn are_friends(conn: &Connection, a: i64, b: i64) -> Result<bool, AppErr> {
    if a == b {
        return Ok(false);
    }
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM friendships WHERE user_lo=?1 AND user_hi=?2",
            params![lo, hi],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn add_friendship(conn: &Connection, a: i64, b: i64, at: i64) -> Result<(), AppErr> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    conn.execute(
        "INSERT OR IGNORE INTO friendships (user_lo, user_hi, created_at) VALUES (?1, ?2, ?3)",
        params![lo, hi, at],
    )?;
    Ok(())
}

pub fn request_friend(conn: &mut Connection, from: i64, to: i64) -> Result<bool, AppErr> {
    if from == to {
        return Err(AppErr::BadRequest("不能添加自己"));
    }
    let target = user_by_id(conn, to)?;
    if target.disabled {
        return Err(AppErr::NotFound);
    }
    if are_friends(conn, from, to)? {
        return Err(AppErr::Conflict("已经是好友"));
    }
    let at = now();
    let tx = conn.transaction()?;
    let reverse: Option<i64> = tx
        .query_row(
            "SELECT id FROM friend_requests WHERE from_user=?1 AND to_user=?2 AND status='pending'",
            params![to, from],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(request_id) = reverse {
        accept_request_tx(&tx, request_id, from, at)?;
        tx.commit()?;
        return Ok(true);
    }
    let existing: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, status FROM friend_requests WHERE from_user=?1 AND to_user=?2",
            params![from, to],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        Some((_, status)) if status == "pending" => return Err(AppErr::Conflict("申请已发送")),
        Some((id, _)) => {
            tx.execute(
                "UPDATE friend_requests SET status='pending', created_at=?1 WHERE id=?2",
                params![at, id],
            )?;
        }
        None => {
            tx.execute(
                "INSERT INTO friend_requests (from_user, to_user, status, created_at) VALUES (?1, ?2, 'pending', ?3)",
                params![from, to, at],
            )?;
        }
    }
    tx.commit()?;
    Ok(false)
}

fn accept_request_tx(
    conn: &Connection,
    request_id: i64,
    actor: i64,
    at: i64,
) -> Result<(i64, i64), AppErr> {
    let (from, to, status): (i64, i64, String) = conn.query_row(
        "SELECT from_user, to_user, status FROM friend_requests WHERE id=?1",
        [request_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if to != actor || status != "pending" {
        return Err(AppErr::NotFound);
    }
    add_friendship(conn, from, to, at)?;
    conn.execute(
        "UPDATE friend_requests SET status='accepted' WHERE id=?1",
        [request_id],
    )?;
    Ok((from, to))
}

pub fn accept_request(
    conn: &mut Connection,
    actor: i64,
    request_id: i64,
) -> Result<(i64, i64), AppErr> {
    let tx = conn.transaction()?;
    let pair = accept_request_tx(&tx, request_id, actor, now())?;
    tx.commit()?;
    Ok(pair)
}

pub fn reject_request(conn: &Connection, actor: i64, request_id: i64) -> Result<i64, AppErr> {
    let (from, to, status): (i64, i64, String) = conn
        .query_row(
            "SELECT from_user, to_user, status FROM friend_requests WHERE id=?1",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or(AppErr::NotFound)?;
    if to != actor || status != "pending" {
        return Err(AppErr::NotFound);
    }
    conn.execute(
        "UPDATE friend_requests SET status='rejected' WHERE id=?1",
        [request_id],
    )?;
    Ok(from)
}

pub fn delete_friend(conn: &mut Connection, actor: i64, other: i64) -> Result<(), AppErr> {
    let (lo, hi) = if actor < other {
        (actor, other)
    } else {
        (other, actor)
    };
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM friendships WHERE user_lo=?1 AND user_hi=?2",
        params![lo, hi],
    )?;
    tx.execute(
        "DELETE FROM friend_requests WHERE (from_user=?1 AND to_user=?2) OR (from_user=?2 AND to_user=?1)",
        params![actor, other],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn invite_usable(conn: &Connection, token: &str) -> Result<(), AppErr> {
    let hash = sha256(token.as_bytes()).to_vec();
    let row = conn
        .query_row(
            "SELECT expires_at, revoked, used_by FROM invites WHERE token_hash=?1",
            [hash],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((expires, revoked, used)) = row else {
        return Err(AppErr::BadRequest("邀请无效"));
    };
    if !invite_open(revoked, used, expires, now()) {
        return Err(AppErr::BadRequest("邀请无效"));
    }
    Ok(())
}

fn invite_open(revoked: i64, used: Option<i64>, expires: i64, at: i64) -> bool {
    revoked == 0 && used.is_none() && expires > at
}

pub fn list_friends(conn: &Connection, user_id: i64) -> Result<Vec<User>, AppErr> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {USER_COLS_U} FROM users u
         JOIN friendships f ON u.id=f.user_lo OR u.id=f.user_hi
         WHERE (f.user_lo=?1 OR f.user_hi=?1) AND u.id!=?1
         ORDER BY u.username_lower"
    ))?;
    let rows = stmt.query_map([user_id], map_user)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn list_requests(conn: &Connection, user_id: i64) -> Result<Vec<FriendRequestRow>, AppErr> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.from_user, r.to_user, r.created_at FROM friend_requests r
         WHERE r.status='pending' AND (r.from_user=?1 OR r.to_user=?1) ORDER BY r.id DESC",
    )?;
    let ids = stmt.query_map([user_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in ids {
        let (id, from, to, created) = row?;
        let other = if from == user_id { to } else { from };
        let user = user_by_id(conn, other)?;
        out.push(FriendRequestRow {
            id,
            incoming: to == user_id,
            user: PublicUser {
                id: user.id,
                uid: user.uid,
                username: user.username,
                display_name: user.display_name,
                online: false,
            },
            created_at: created,
        });
    }
    Ok(out)
}

fn member_ids(conn: &Connection, conversation: i64) -> Result<Vec<i64>, AppErr> {
    let mut stmt = conn.prepare("SELECT user_id FROM members WHERE conversation_id=?1")?;
    let rows = stmt.query_map([conversation], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

fn is_member(conn: &Connection, conversation: i64, user: i64) -> Result<Option<String>, AppErr> {
    Ok(conn
        .query_row(
            "SELECT role FROM members WHERE conversation_id=?1 AND user_id=?2",
            params![conversation, user],
            |row| row.get(0),
        )
        .optional()?)
}

fn conversation_kind(conn: &Connection, id: i64) -> Result<(String, i64, Option<Vec<u8>>), AppErr> {
    conn.query_row(
        "SELECT kind, retention_secs, name_enc FROM conversations WHERE id=?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()?
    .ok_or(AppErr::NotFound)
}

fn require_reader(conn: &Connection, conversation: i64, user: i64) -> Result<(), AppErr> {
    if is_member(conn, conversation, user)?.is_none() {
        return Err(AppErr::NotFound);
    }
    Ok(())
}

fn require_sender(conn: &Connection, conversation: i64, user: i64) -> Result<i64, AppErr> {
    require_reader(conn, conversation, user)?;
    let (kind, retention, _) = conversation_kind(conn, conversation)?;
    if kind == "dm" {
        let others: Vec<i64> = member_ids(conn, conversation)?
            .into_iter()
            .filter(|id| *id != user)
            .collect();
        if others.len() != 1 || !are_friends(conn, user, others[0])? {
            return Err(AppErr::Forbidden("需要先成为好友"));
        }
        if user_by_id(conn, others[0])?.disabled {
            return Err(AppErr::Forbidden("对方账号不可用"));
        }
    }
    Ok(retention)
}

pub fn create_dm(conn: &mut Connection, user: i64, other: i64) -> Result<i64, AppErr> {
    if user == other {
        return Err(AppErr::BadRequest("不能和自己私聊"));
    }
    if user_by_id(conn, other)?.disabled {
        return Err(AppErr::NotFound);
    }
    if !are_friends(conn, user, other)? {
        return Err(AppErr::Forbidden("需要先成为好友"));
    }
    let (lo, hi) = if user < other {
        (user, other)
    } else {
        (other, user)
    };
    let pair = format!("{lo}:{hi}");
    let tx = conn.transaction()?;
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM conversations WHERE dm_pair=?1",
            [&pair],
            |row| row.get(0),
        )
        .optional()?
    {
        tx.commit()?;
        return Ok(id);
    }
    let at = now();
    tx.execute(
        "INSERT INTO conversations (kind, dm_pair, retention_secs, created_by, created_at) VALUES ('dm', ?1, ?2, ?3, ?4)",
        params![pair, RET_MONTH, user, at],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO members (conversation_id, user_id, role, joined_at) VALUES (?1, ?2, 'member', ?3), (?1, ?4, 'member', ?3)",
        params![id, lo, at, hi],
    )?;
    tx.commit()?;
    Ok(id)
}

pub fn create_group(
    conn: &mut Connection,
    key: &Key,
    owner: i64,
    name: &str,
    members: &[i64],
) -> Result<i64, AppErr> {
    let name = util::clean_group_name(name).map_err(AppErr::BadRequest)?;
    let mut unique = Vec::new();
    for member in members {
        if *member == owner || unique.contains(member) {
            continue;
        }
        if !are_friends(conn, owner, *member)? {
            return Err(AppErr::Forbidden("只能拉好友进群"));
        }
        if user_by_id(conn, *member)?.disabled {
            return Err(AppErr::BadRequest("对方账号不可用"));
        }
        unique.push(*member);
    }
    if (unique.len() as i64) + 1 > GROUP_LIMIT {
        return Err(AppErr::BadRequest("群人数不能超过 100"));
    }
    let at = now();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO conversations (kind, retention_secs, created_by, created_at) VALUES ('group', ?1, ?2, ?3)",
        params![RET_MONTH, owner, at],
    )?;
    let id = tx.last_insert_rowid();
    let sealed = crypto::seal(key, name_aad(id).as_bytes(), name.as_bytes())?;
    tx.execute(
        "UPDATE conversations SET name_enc=?1 WHERE id=?2",
        params![sealed, id],
    )?;
    tx.execute(
        "INSERT INTO members (conversation_id, user_id, role, joined_at) VALUES (?1, ?2, 'owner', ?3)",
        params![id, owner, at],
    )?;
    for member in &unique {
        tx.execute(
            "INSERT INTO members (conversation_id, user_id, role, joined_at) VALUES (?1, ?2, 'member', ?3)",
            params![id, member, at],
        )?;
    }
    let owner_name = user_by_id(&tx, owner)?.display_name;
    insert_message(
        &tx,
        key,
        id,
        owner,
        "system",
        Some(&format!("{owner_name} 创建了群聊")),
        None,
        at,
        RET_MONTH,
    )?;
    tx.commit()?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn insert_message(
    conn: &Connection,
    key: &Key,
    conversation: i64,
    sender: i64,
    kind: &str,
    body: Option<&str>,
    image_id: Option<i64>,
    at: i64,
    retention: i64,
) -> Result<i64, AppErr> {
    let sealed = match body {
        Some(text) => Some(crypto::seal(
            key,
            aad(conversation, sender, at, kind).as_bytes(),
            text.as_bytes(),
        )?),
        None => None,
    };
    conn.execute(
        "INSERT INTO messages (conversation_id, sender_id, kind, body_enc, image_id, created_at, expires_at, recalled)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
        params![conversation, sender, kind, sealed, image_id, at, at.saturating_add(retention)],
    )?;
    Ok(conn.last_insert_rowid())
}

fn message_out(conn: &Connection, key: &Key, viewer: i64, id: i64) -> Result<MessageOut, AppErr> {
    let row = conn
        .query_row(
            "SELECT m.id, m.conversation_id, m.sender_id, u.username, u.display_name, m.kind, m.body_enc, m.image_id, i.width, i.height, m.recalled, m.created_at
             FROM messages m
             LEFT JOIN users u ON u.id=m.sender_id
             LEFT JOIN images i ON i.id=m.image_id
             WHERE m.id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, i64>(10)? != 0,
                    row.get::<_, i64>(11)?,
                ))
            },
        )
        .optional()?
        .ok_or(AppErr::NotFound)?;
    let (
        id,
        conversation,
        sender,
        username,
        display,
        kind,
        body_enc,
        image_id,
        width,
        height,
        recalled,
        created,
    ) = row;
    let body = if recalled {
        None
    } else if let Some(packed) = body_enc {
        let sender_id = sender.unwrap_or(0);
        match crypto::open(
            key,
            aad(conversation, sender_id, created, &kind).as_bytes(),
            &packed,
        ) {
            Ok(plain) => Some(String::from_utf8(plain).unwrap_or_else(|_| "［无法显示］".into())),
            Err(_) => Some("［无法解密］".into()),
        }
    } else {
        None
    };
    Ok(MessageOut {
        id,
        conversation_id: conversation,
        sender_id: sender,
        sender_username: username,
        sender_display_name: display.unwrap_or_else(|| "已注销用户".into()),
        kind,
        body,
        image_id: if recalled { None } else { image_id },
        width: if recalled { None } else { width },
        height: if recalled { None } else { height },
        recalled,
        created_at: created,
        mine: sender == Some(viewer),
    })
}

pub fn post_text(
    conn: &mut Connection,
    key: &Key,
    user: i64,
    conversation: i64,
    text: &str,
) -> Result<Posted, AppErr> {
    let text = util::clean_text(text).map_err(AppErr::BadRequest)?;
    let tx = conn.transaction()?;
    let retention = require_sender(&tx, conversation, user)?;
    let at = now();
    let id = insert_message(
        &tx,
        key,
        conversation,
        user,
        "text",
        Some(&text),
        None,
        at,
        retention,
    )?;
    let members = member_ids(&tx, conversation)?;
    let message = message_out(&tx, key, user, id)?;
    tx.commit()?;
    Ok(Posted { message, members })
}

pub struct NewImage {
    pub full_name: String,
    pub thumb_name: String,
    pub width: i64,
    pub height: i64,
    pub bytes: i64,
}

pub fn post_image(
    conn: &mut Connection,
    key: &Key,
    user: i64,
    conversation: i64,
    image: NewImage,
) -> Result<Posted, AppErr> {
    let tx = conn.transaction()?;
    let retention = require_sender(&tx, conversation, user)?;
    let at = now();
    tx.execute(
        "INSERT INTO images (owner_id, conversation_id, file_name, thumb_name, width, height, bytes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![user, conversation, image.full_name, image.thumb_name, image.width, image.height, image.bytes, at],
    )?;
    let image_id = tx.last_insert_rowid();
    let id = insert_message(
        &tx,
        key,
        conversation,
        user,
        "image",
        None,
        Some(image_id),
        at,
        retention,
    )?;
    let members = member_ids(&tx, conversation)?;
    let message = message_out(&tx, key, user, id)?;
    tx.commit()?;
    Ok(Posted { message, members })
}

pub fn recall(
    conn: &mut Connection,
    key: &Key,
    user: i64,
    message_id: i64,
) -> Result<(MessageOut, Vec<String>, Vec<i64>), AppErr> {
    let tx = conn.transaction()?;
    let (conversation, sender, kind, image_id, recalled): (
        i64,
        Option<i64>,
        String,
        Option<i64>,
        i64,
    ) = tx
        .query_row(
            "SELECT conversation_id, sender_id, kind, image_id, recalled FROM messages WHERE id=?1",
            [message_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?
        .ok_or(AppErr::NotFound)?;
    require_reader(&tx, conversation, user)?;
    if sender != Some(user) || kind == "system" {
        return Err(AppErr::Forbidden("只能撤回自己的消息"));
    }
    let mut files = Vec::new();
    if recalled == 0 {
        if let Some(image_id) = image_id {
            files.extend(image_names(&tx, image_id)?);
            tx.execute("DELETE FROM images WHERE id=?1", [image_id])?;
        }
        tx.execute(
            "UPDATE messages SET body_enc=NULL, image_id=NULL, recalled=1 WHERE id=?1",
            [message_id],
        )?;
    }
    let members = member_ids(&tx, conversation)?;
    let message = message_out(&tx, key, user, message_id)?;
    tx.commit()?;
    Ok((message, files, members))
}

fn image_names(conn: &Connection, image_id: i64) -> Result<Vec<String>, AppErr> {
    let names = conn
        .query_row(
            "SELECT file_name, thumb_name FROM images WHERE id=?1",
            [image_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(match names {
        Some((full, thumb)) => vec![full, thumb],
        None => Vec::new(),
    })
}

pub fn hide_message(conn: &Connection, user: i64, message_id: i64) -> Result<(), AppErr> {
    let conversation: i64 = conn
        .query_row(
            "SELECT conversation_id FROM messages WHERE id=?1",
            [message_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(AppErr::NotFound)?;
    require_reader(conn, conversation, user)?;
    conn.execute(
        "INSERT OR IGNORE INTO message_hidden (message_id, user_id) VALUES (?1, ?2)",
        params![message_id, user],
    )?;
    Ok(())
}

pub fn list_messages(
    conn: &Connection,
    key: &Key,
    user: i64,
    conversation: i64,
    before: Option<i64>,
    limit: i64,
) -> Result<Vec<MessageOut>, AppErr> {
    require_reader(conn, conversation, user)?;
    let limit = limit.clamp(1, 100);
    let mut stmt = conn.prepare(
        "SELECT m.id FROM messages m
         WHERE m.conversation_id=?1 AND (?2 = 0 OR m.id < ?2)
           AND m.expires_at > ?3
           AND NOT EXISTS (SELECT 1 FROM message_hidden h WHERE h.message_id=m.id AND h.user_id=?4)
         ORDER BY m.id DESC LIMIT ?5",
    )?;
    let ids = stmt.query_map(
        params![conversation, before.unwrap_or(0), now(), user, limit],
        |row| row.get::<_, i64>(0),
    )?;
    let mut messages = Vec::new();
    for id in ids {
        messages.push(message_out(conn, key, user, id?)?);
    }
    messages.reverse();
    Ok(messages)
}

pub fn mark_read(
    conn: &Connection,
    user: i64,
    conversation: i64,
    message_id: i64,
) -> Result<Vec<i64>, AppErr> {
    require_reader(conn, conversation, user)?;
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM messages WHERE id=?1 AND conversation_id=?2",
            params![message_id, conversation],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(AppErr::NotFound);
    }
    conn.execute(
        "UPDATE members SET last_read_id=?1 WHERE conversation_id=?2 AND user_id=?3 AND (last_read_id IS NULL OR last_read_id < ?1)",
        params![message_id, conversation, user],
    )?;
    member_ids(conn, conversation)
}

fn group_title(key: &Key, id: i64, packed: Option<Vec<u8>>) -> String {
    let Some(packed) = packed else {
        return "群聊".into();
    };
    crypto::open(key, name_aad(id).as_bytes(), &packed)
        .ok()
        .and_then(|plain| String::from_utf8(plain).ok())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "群聊".into())
}

fn dm_title(conn: &Connection, conversation: i64, user: i64) -> Result<String, AppErr> {
    let others = member_ids(conn, conversation)?;
    for other in others {
        if other != user {
            return Ok(user_by_id(conn, other)
                .map(|user| user.display_name)
                .unwrap_or_else(|_| "私聊".into()));
        }
    }
    Ok("私聊".into())
}

fn unread(conn: &Connection, conversation: i64, user: i64) -> Result<i64, AppErr> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM messages m
         JOIN members mem ON mem.conversation_id=m.conversation_id AND mem.user_id=?2
         WHERE m.conversation_id=?1 AND m.id > COALESCE(mem.last_read_id, 0)
           AND m.expires_at > ?3
           AND IFNULL(m.sender_id, 0) != ?2 AND m.recalled=0
           AND NOT EXISTS (SELECT 1 FROM message_hidden h WHERE h.message_id=m.id AND h.user_id=?2)",
        params![conversation, user, now()],
        |row| row.get(0),
    )?)
}

fn last_visible(
    conn: &Connection,
    key: &Key,
    conversation: i64,
    user: i64,
) -> Result<Option<MessageOut>, AppErr> {
    let id: Option<i64> = conn
        .query_row(
            "SELECT m.id FROM messages m
             WHERE m.conversation_id=?1 AND m.expires_at > ?2
               AND NOT EXISTS (SELECT 1 FROM message_hidden h WHERE h.message_id=m.id AND h.user_id=?3)
             ORDER BY m.id DESC LIMIT 1",
            params![conversation, now(), user],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        Some(id) => Ok(Some(message_out(conn, key, user, id)?)),
        None => Ok(None),
    }
}

fn preview_of(message: &MessageOut) -> Option<String> {
    if message.recalled {
        return Some("消息已撤回".into());
    }
    match message.kind.as_str() {
        "image" => Some("图片".into()),
        "text" | "system" => message.body.as_deref().map(|text| preview(text, 80)),
        _ => None,
    }
}

fn conversation_out(
    conn: &Connection,
    key: &Key,
    user: i64,
    id: i64,
    with_members: bool,
) -> Result<ConversationOut, AppErr> {
    let role = is_member(conn, id, user)?.ok_or(AppErr::NotFound)?;
    let (kind, retention, name) = conversation_kind(conn, id)?;
    let title = if kind == "dm" {
        dm_title(conn, id, user)?
    } else {
        group_title(key, id, name)
    };
    let last = last_visible(conn, key, id, user)?;
    let can_send = if kind == "dm" {
        let others = member_ids(conn, id)?;
        others
            .into_iter()
            .filter(|other| *other != user)
            .any(|other| {
                are_friends(conn, user, other).unwrap_or(false)
                    && user_by_id(conn, other).is_ok_and(|user| !user.disabled)
            })
    } else {
        true
    };
    let members = if with_members {
        Some(member_outs(conn, id)?)
    } else {
        None
    };
    Ok(ConversationOut {
        id,
        kind,
        title,
        retention,
        unread: unread(conn, id, user)?,
        role,
        can_send,
        last_preview: last.as_ref().and_then(preview_of),
        last_at: last.as_ref().map(|message| message.created_at),
        members,
    })
}

fn member_outs(conn: &Connection, conversation: i64) -> Result<Vec<MemberOut>, AppErr> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.uid, u.username, u.display_name, mem.role
         FROM members mem JOIN users u ON u.id=mem.user_id
         WHERE mem.conversation_id=?1
         ORDER BY mem.role DESC, u.username_lower",
    )?;
    let rows = stmt.query_map([conversation], |row| {
        Ok(MemberOut {
            id: row.get(0)?,
            uid: row.get(1)?,
            username: row.get(2)?,
            display_name: row.get(3)?,
            role: row.get(4)?,
            online: false,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn list_conversations(
    conn: &Connection,
    key: &Key,
    user: i64,
) -> Result<Vec<ConversationOut>, AppErr> {
    let mut stmt = conn.prepare(
        "SELECT c.id FROM conversations c JOIN members m ON m.conversation_id=c.id AND m.user_id=?1
         ORDER BY IFNULL((SELECT MAX(id) FROM messages WHERE conversation_id=c.id), 0) DESC, c.id DESC",
    )?;
    let ids = stmt.query_map([user], |row| row.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for id in ids {
        out.push(conversation_out(conn, key, user, id?, false)?);
    }
    Ok(out)
}

pub fn conversation(
    conn: &Connection,
    key: &Key,
    user: i64,
    id: i64,
) -> Result<ConversationOut, AppErr> {
    conversation_out(conn, key, user, id, true)
}

pub fn set_retention(
    conn: &mut Connection,
    user: i64,
    conversation: i64,
    retention: i64,
) -> Result<Vec<i64>, AppErr> {
    if !util::retention_ok(retention) {
        return Err(AppErr::BadRequest(
            "保留时间只能是 1 小时、1 天、7 天或 30 天",
        ));
    }
    let role = is_member(conn, conversation, user)?.ok_or(AppErr::NotFound)?;
    let (kind, _, _) = conversation_kind(conn, conversation)?;
    if kind == "group" && role != "owner" {
        return Err(AppErr::Forbidden("只有群主可以修改保留时间"));
    }
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE conversations SET retention_secs=?1 WHERE id=?2",
        params![retention, conversation],
    )?;
    tx.execute(
        "UPDATE messages SET expires_at=MIN(expires_at, created_at + ?1) WHERE conversation_id=?2",
        params![retention, conversation],
    )?;
    let members = member_ids(&tx, conversation)?;
    tx.commit()?;
    Ok(members)
}

pub fn rename_group(
    conn: &Connection,
    key: &Key,
    user: i64,
    conversation: i64,
    name: &str,
) -> Result<Vec<i64>, AppErr> {
    let name = util::clean_group_name(name).map_err(AppErr::BadRequest)?;
    let role = is_member(conn, conversation, user)?.ok_or(AppErr::NotFound)?;
    let (kind, _, _) = conversation_kind(conn, conversation)?;
    if kind != "group" || role != "owner" {
        return Err(AppErr::Forbidden("只有群主可以改名"));
    }
    let sealed = crypto::seal(key, name_aad(conversation).as_bytes(), name.as_bytes())?;
    conn.execute(
        "UPDATE conversations SET name_enc=?1 WHERE id=?2",
        params![sealed, conversation],
    )?;
    member_ids(conn, conversation)
}

pub fn add_member(
    conn: &mut Connection,
    key: &Key,
    actor: i64,
    conversation: i64,
    target: i64,
) -> Result<Vec<i64>, AppErr> {
    if is_member(conn, conversation, actor)?.is_none() {
        return Err(AppErr::NotFound);
    }
    let (kind, retention, _) = conversation_kind(conn, conversation)?;
    if kind != "group" {
        return Err(AppErr::BadRequest("私聊不能加人"));
    }
    if !are_friends(conn, actor, target)? {
        return Err(AppErr::Forbidden("只能拉自己的好友进群"));
    }
    if user_by_id(conn, target)?.disabled {
        return Err(AppErr::BadRequest("对方账号不可用"));
    }
    if is_member(conn, conversation, target)?.is_some() {
        return Err(AppErr::Conflict("对方已在群里"));
    }
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM members WHERE conversation_id=?1",
        [conversation],
        |row| row.get(0),
    )?;
    if count >= GROUP_LIMIT {
        return Err(AppErr::BadRequest("群人数不能超过 100"));
    }
    let at = now();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO members (conversation_id, user_id, role, joined_at, last_read_id)
         VALUES (?1, ?2, 'member', ?3, (SELECT IFNULL(MAX(id), 0) FROM messages WHERE conversation_id=?1))",
        params![conversation, target, at],
    )?;
    let name = user_by_id(&tx, target)?.display_name;
    insert_message(
        &tx,
        key,
        conversation,
        actor,
        "system",
        Some(&format!("{name} 加入了群聊")),
        None,
        at,
        retention,
    )?;
    let members = member_ids(&tx, conversation)?;
    tx.commit()?;
    Ok(members)
}

pub fn kick_member(
    conn: &mut Connection,
    key: &Key,
    actor: i64,
    conversation: i64,
    target: i64,
) -> Result<Vec<i64>, AppErr> {
    let role = is_member(conn, conversation, actor)?.ok_or(AppErr::NotFound)?;
    if role != "owner" {
        return Err(AppErr::Forbidden("只有群主可以移出成员"));
    }
    if actor == target {
        return Err(AppErr::BadRequest("请使用退出"));
    }
    let target_role = is_member(conn, conversation, target)?.ok_or(AppErr::NotFound)?;
    if target_role == "owner" {
        return Err(AppErr::Forbidden("不能移出群主"));
    }
    let (kind, retention, _) = conversation_kind(conn, conversation)?;
    if kind != "group" {
        return Err(AppErr::BadRequest("私聊不能移出成员"));
    }
    let at = now();
    let tx = conn.transaction()?;
    let name = user_by_id(&tx, target)?.display_name;
    tx.execute(
        "DELETE FROM members WHERE conversation_id=?1 AND user_id=?2",
        params![conversation, target],
    )?;
    insert_message(
        &tx,
        key,
        conversation,
        actor,
        "system",
        Some(&format!("{name} 被移出群聊")),
        None,
        at,
        retention,
    )?;
    let members = member_ids(&tx, conversation)?;
    tx.commit()?;
    Ok(members)
}

pub fn transfer(
    conn: &mut Connection,
    key: &Key,
    actor: i64,
    conversation: i64,
    target: i64,
) -> Result<Vec<i64>, AppErr> {
    let role = is_member(conn, conversation, actor)?.ok_or(AppErr::NotFound)?;
    if role != "owner" || actor == target {
        return Err(AppErr::Forbidden("只有群主可以转让"));
    }
    if is_member(conn, conversation, target)?.is_none() {
        return Err(AppErr::NotFound);
    }
    let (kind, retention, _) = conversation_kind(conn, conversation)?;
    if kind != "group" {
        return Err(AppErr::BadRequest("私聊没有群主"));
    }
    let at = now();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE members SET role='member' WHERE conversation_id=?1 AND user_id=?2",
        params![conversation, actor],
    )?;
    tx.execute(
        "UPDATE members SET role='owner' WHERE conversation_id=?1 AND user_id=?2",
        params![conversation, target],
    )?;
    let name = user_by_id(&tx, target)?.display_name;
    insert_message(
        &tx,
        key,
        conversation,
        actor,
        "system",
        Some(&format!("群主已转让给 {name}")),
        None,
        at,
        retention,
    )?;
    let members = member_ids(&tx, conversation)?;
    tx.commit()?;
    Ok(members)
}

pub fn leave(
    conn: &mut Connection,
    key: &Key,
    user: i64,
    conversation: i64,
) -> Result<(Vec<i64>, Vec<String>), AppErr> {
    let role = is_member(conn, conversation, user)?.ok_or(AppErr::NotFound)?;
    let (kind, retention, _) = conversation_kind(conn, conversation)?;
    if kind != "group" {
        return Err(AppErr::BadRequest("私聊不能退出，解除好友后将无法继续发送"));
    }
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM members WHERE conversation_id=?1",
        [conversation],
        |row| row.get(0),
    )?;
    if role == "owner" && count > 1 {
        return Err(AppErr::BadRequest("请先转让群主"));
    }
    if count <= 1 {
        let tx = conn.transaction()?;
        let files = delete_conversation(&tx, conversation)?;
        tx.commit()?;
        return Ok((Vec::new(), files));
    }
    let at = now();
    let tx = conn.transaction()?;
    let name = user_by_id(&tx, user)?.display_name;
    tx.execute(
        "DELETE FROM members WHERE conversation_id=?1 AND user_id=?2",
        params![conversation, user],
    )?;
    insert_message(
        &tx,
        key,
        conversation,
        user,
        "system",
        Some(&format!("{name} 退出了群聊")),
        None,
        at,
        retention,
    )?;
    let members = member_ids(&tx, conversation)?;
    tx.commit()?;
    Ok((members, Vec::new()))
}

fn delete_conversation(conn: &Connection, id: i64) -> Result<Vec<String>, AppErr> {
    let mut stmt =
        conn.prepare("SELECT file_name, thumb_name FROM images WHERE conversation_id=?1")?;
    let rows = stmt.query_map([id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut files = Vec::new();
    for row in rows {
        let (full, thumb) = row?;
        files.push(full);
        files.push(thumb);
    }
    drop(stmt);
    delete_in(
        conn,
        "DELETE FROM message_hidden WHERE message_id IN",
        &message_ids(conn, id)?,
    )?;
    conn.execute("DELETE FROM messages WHERE conversation_id=?1", [id])?;
    conn.execute("DELETE FROM images WHERE conversation_id=?1", [id])?;
    conn.execute("DELETE FROM members WHERE conversation_id=?1", [id])?;
    conn.execute("DELETE FROM conversations WHERE id=?1", [id])?;
    Ok(files)
}

fn message_ids(conn: &Connection, conversation: i64) -> Result<Vec<i64>, AppErr> {
    let mut stmt = conn.prepare("SELECT id FROM messages WHERE conversation_id=?1")?;
    let rows = stmt.query_map([conversation], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

fn delete_in(conn: &Connection, prefix: &str, ids: &[i64]) -> Result<(), AppErr> {
    if ids.is_empty() {
        return Ok(());
    }
    let marks = vec!["?"; ids.len()].join(",");
    conn.execute(&format!("{prefix} ({marks})"), params_from_iter(ids.iter()))?;
    Ok(())
}

pub fn image_name_for_member(
    conn: &Connection,
    user: i64,
    image_id: i64,
    thumb: bool,
) -> Result<String, AppErr> {
    let (conversation, full, thumb_name): (i64, String, String) = conn
        .query_row(
            "SELECT conversation_id, file_name, thumb_name FROM images WHERE id=?1",
            [image_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or(AppErr::NotFound)?;
    require_reader(conn, conversation, user)?;
    let live: Option<i64> = conn
        .query_row(
            "SELECT id FROM messages WHERE image_id=?1 AND conversation_id=?2 AND expires_at>?3 LIMIT 1",
            params![image_id, conversation, now()],
            |row| row.get(0),
        )
        .optional()?;
    if live.is_none() {
        return Err(AppErr::NotFound);
    }
    Ok(if thumb { thumb_name } else { full })
}

pub fn sweep(conn: &mut Connection, at: i64) -> Result<Vec<String>, AppErr> {
    let mut files = Vec::new();
    for _ in 0..20 {
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "SELECT m.id, i.file_name, i.thumb_name FROM messages m
             LEFT JOIN images i ON i.id=m.image_id
             WHERE m.expires_at<=?1 LIMIT 200",
        )?;
        let rows = stmt.query_map([at], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut ids = Vec::new();
        for row in rows {
            let (id, full, thumb) = row?;
            ids.push(id);
            if let Some(name) = full {
                files.push(name);
            }
            if let Some(name) = thumb {
                files.push(name);
            }
        }
        drop(stmt);
        if ids.is_empty() {
            tx.commit()?;
            break;
        }
        delete_in(&tx, "DELETE FROM message_hidden WHERE message_id IN", &ids)?;
        delete_in(&tx, "DELETE FROM messages WHERE id IN", &ids)?;
        tx.execute(
            "DELETE FROM images WHERE id NOT IN (SELECT image_id FROM messages WHERE image_id IS NOT NULL) AND conversation_id IN (SELECT id FROM conversations)",
            [],
        )?;
        tx.commit()?;
    }
    Ok(files)
}

pub fn referenced_images(conn: &Connection) -> Result<HashSet<String>, AppErr> {
    let mut stmt = conn.prepare("SELECT file_name, thumb_name FROM images")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut names = HashSet::new();
    for row in rows {
        let (full, thumb) = row?;
        names.insert(full);
        names.insert(thumb);
    }
    Ok(names)
}

pub fn save_push(
    conn: &Connection,
    user: i64,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
) -> Result<(), AppErr> {
    if !crate::push::endpoint_allowed(endpoint) {
        return Err(AppErr::BadRequest("推送订阅无效"));
    }
    if p256dh.len() > 256 || auth.len() > 256 {
        return Err(AppErr::BadRequest("推送订阅无效"));
    }
    let at = now();
    conn.execute(
        "INSERT INTO push_subs (user_id, endpoint, p256dh, auth, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(endpoint) DO UPDATE SET user_id=excluded.user_id, p256dh=excluded.p256dh, auth=excluded.auth",
        params![user, endpoint, p256dh, auth, at],
    )?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM push_subs WHERE user_id=?1",
        [user],
        |row| row.get(0),
    )?;
    if count > 5 {
        conn.execute(
            "DELETE FROM push_subs WHERE id IN (SELECT id FROM push_subs WHERE user_id=?1 ORDER BY id ASC LIMIT ?2)",
            params![user, count - 5],
        )?;
    }
    Ok(())
}

pub fn delete_push(conn: &Connection, user: i64, endpoint: &str) -> Result<(), AppErr> {
    conn.execute(
        "DELETE FROM push_subs WHERE user_id=?1 AND endpoint=?2",
        params![user, endpoint],
    )?;
    Ok(())
}

pub fn delete_push_endpoint(conn: &Connection, endpoint: &str) -> Result<(), AppErr> {
    conn.execute("DELETE FROM push_subs WHERE endpoint=?1", [endpoint])?;
    Ok(())
}

pub fn push_subs_for(
    conn: &Connection,
    users: &[i64],
) -> Result<Vec<crate::push::PushSub>, AppErr> {
    if users.is_empty() {
        return Ok(Vec::new());
    }
    let marks = vec!["?"; users.len()].join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT user_id, endpoint, p256dh, auth FROM push_subs WHERE user_id IN ({marks})"
    ))?;
    let rows = stmt.query_map(params_from_iter(users.iter()), |row| {
        Ok(crate::push::PushSub {
            user_id: row.get(0)?,
            endpoint: row.get(1)?,
            p256dh: row.get(2)?,
            auth: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn admin_count(conn: &Connection) -> Result<i64, AppErr> {
    Ok(
        conn.query_row("SELECT COUNT(*) FROM users WHERE is_admin=1", [], |row| {
            row.get(0)
        })?,
    )
}

pub fn set_disabled(conn: &Connection, user_id: i64, disabled: bool) -> Result<(), AppErr> {
    conn.execute(
        "UPDATE users SET disabled=?1 WHERE id=?2",
        params![i64::from(disabled), user_id],
    )?;
    if conn.changes() != 1 {
        return Err(AppErr::NotFound);
    }
    if disabled {
        delete_user_sessions(conn, user_id)?;
        conn.execute("DELETE FROM push_subs WHERE user_id=?1", [user_id])?;
    }
    Ok(())
}

pub fn set_admin(conn: &Connection, user_id: i64, admin: bool) -> Result<(), AppErr> {
    let user = user_by_id(conn, user_id)?;
    if admin && !user.totp_enabled {
        return Err(AppErr::BadRequest("对方需要先开启两步验证"));
    }
    if !admin && user.is_admin && admin_count(conn)? <= 1 {
        return Err(AppErr::Forbidden("不能取消最后一位管理员"));
    }
    conn.execute(
        "UPDATE users SET is_admin=?1 WHERE id=?2",
        params![i64::from(admin), user_id],
    )?;
    if !admin {
        conn.execute(
            "DELETE FROM sessions WHERE user_id=?1 AND kind='admin'",
            [user_id],
        )?;
    }
    Ok(())
}

pub fn delete_user(conn: &mut Connection, user_id: i64) -> Result<Vec<String>, AppErr> {
    let user = user_by_id(conn, user_id)?;
    if user.is_admin && admin_count(conn)? <= 1 {
        return Err(AppErr::Forbidden("不能删除最后一位管理员"));
    }
    let tx = conn.transaction()?;
    let mut files = Vec::new();
    let mut dm_stmt = tx.prepare(
        "SELECT c.id FROM conversations c JOIN members m ON m.conversation_id=c.id WHERE c.kind='dm' AND m.user_id=?1",
    )?;
    let dm_ids: Vec<i64> = dm_stmt
        .query_map([user_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(dm_stmt);
    for id in dm_ids {
        files.extend(delete_conversation(&tx, id)?);
    }
    let mut owned =
        tx.prepare("SELECT conversation_id FROM members WHERE user_id=?1 AND role='owner'")?;
    let owned_ids: Vec<i64> = owned
        .query_map([user_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(owned);
    for id in owned_ids {
        let others: Vec<i64> = member_ids(&tx, id)?
            .into_iter()
            .filter(|member| *member != user_id)
            .collect();
        if others.is_empty() {
            files.extend(delete_conversation(&tx, id)?);
        } else {
            tx.execute(
                "UPDATE members SET role='member' WHERE conversation_id=?1 AND user_id=?2",
                params![id, user_id],
            )?;
            tx.execute(
                "UPDATE members SET role='owner' WHERE conversation_id=?1 AND user_id=?2",
                params![id, others[0]],
            )?;
        }
    }
    let mut img = tx.prepare("SELECT file_name, thumb_name FROM images WHERE owner_id=?1")?;
    for row in img.query_map([user_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (full, thumb) = row?;
        files.push(full);
        files.push(thumb);
    }
    drop(img);
    tx.execute(
        "DELETE FROM message_hidden WHERE user_id=?1 OR message_id IN (SELECT id FROM messages WHERE sender_id=?1)",
        [user_id],
    )?;
    tx.execute("DELETE FROM messages WHERE sender_id=?1", [user_id])?;
    tx.execute("DELETE FROM images WHERE owner_id=?1", [user_id])?;
    tx.execute("DELETE FROM members WHERE user_id=?1", [user_id])?;
    tx.execute(
        "DELETE FROM friendships WHERE user_lo=?1 OR user_hi=?1",
        [user_id],
    )?;
    tx.execute(
        "DELETE FROM friend_requests WHERE from_user=?1 OR to_user=?1",
        [user_id],
    )?;
    tx.execute("DELETE FROM sessions WHERE user_id=?1", [user_id])?;
    tx.execute("DELETE FROM push_subs WHERE user_id=?1", [user_id])?;
    tx.execute(
        "UPDATE invites SET created_by=NULL WHERE created_by=?1",
        [user_id],
    )?;
    tx.execute(
        "UPDATE invites SET used_by=NULL WHERE used_by=?1",
        [user_id],
    )?;
    tx.execute("DELETE FROM users WHERE id=?1", [user_id])?;
    tx.commit()?;
    Ok(files)
}

pub fn list_users(conn: &Connection) -> Result<Vec<User>, AppErr> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {USER_COLS} FROM users ORDER BY id ASC LIMIT 500"
    ))?;
    let rows = stmt.query_map([], map_user)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn conv_stats(conn: &Connection) -> Result<Vec<ConvStat>, AppErr> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.kind, c.retention_secs, c.created_at,
            (SELECT COUNT(*) FROM members m WHERE m.conversation_id=c.id),
            (SELECT COUNT(*) FROM messages m WHERE m.conversation_id=c.id),
            (SELECT COALESCE(SUM(bytes), 0) FROM images i WHERE i.conversation_id=c.id)
         FROM conversations c ORDER BY c.id DESC LIMIT 500",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ConvStat {
            id: row.get(0)?,
            kind: row.get(1)?,
            retention: row.get(2)?,
            created_at: row.get(3)?,
            members: row.get(4)?,
            messages: row.get(5)?,
            image_bytes: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn counts(conn: &Connection) -> Result<(i64, i64, i64, i64), AppErr> {
    let users = user_count(conn)?;
    let conversations: i64 =
        conn.query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))?;
    let messages: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
    let images: i64 = conn.query_row("SELECT COALESCE(SUM(bytes), 0) FROM images", [], |row| {
        row.get(0)
    })?;
    Ok((users, conversations, messages, images))
}

pub struct AuditRow {
    pub id: i64,
    pub actor_id: Option<i64>,
    pub action: String,
    pub detail: String,
    pub created_at: i64,
}

pub fn list_audit(conn: &Connection) -> Result<Vec<AuditRow>, AppErr> {
    let mut stmt = conn.prepare(
        "SELECT id, actor_id, action, detail, created_at FROM audit_log ORDER BY id DESC LIMIT 200",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AuditRow {
            id: row.get(0)?,
            actor_id: row.get(1)?,
            action: row.get(2)?,
            detail: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppErr::from)
}

pub fn vacuum(conn: &Connection) -> Result<(), AppErr> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
    Ok(())
}

pub fn checkpoint(conn: &Connection) -> Result<(), AppErr> {
    conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
    Ok(())
}

#[cfg(test)]
pub fn force_expiry(conn: &Connection, message_id: i64, at: i64) -> Result<(), AppErr> {
    conn.execute(
        "UPDATE messages SET expires_at=?1 WHERE id=?2",
        params![at, message_id],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn touch_now() -> i64 {
    now()
}
