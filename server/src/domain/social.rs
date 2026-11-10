use sqlx::FromRow;

use crate::auth::AuthUser;
use crate::domain;
use crate::dto::{FriendRequestDto, FriendRequestLists, PublicUser, ServerEvent};
use crate::error::AppError;
use crate::state::AppState;
use crate::util::now_ms;

#[derive(FromRow)]
struct UserLite {
    id: i64,
    uid: String,
    username: String,
}

pub async fn are_friends(state: &AppState, a: i64, b: i64) -> Result<bool, AppError> {
    if a == b {
        return Ok(false);
    }
    let (left, right) = ordered(a, b);
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM friendships WHERE user_a = ? AND user_b = ?")
            .bind(left)
            .bind(right)
            .fetch_optional(&state.db)
            .await?;
    Ok(exists.is_some())
}

fn ordered(a: i64, b: i64) -> (i64, i64) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

async fn relation(state: &AppState, me: i64, other: i64) -> Result<String, AppError> {
    if me == other {
        return Ok("self".into());
    }
    if are_friends(state, me, other).await? {
        return Ok("friends".into());
    }
    let outgoing: Option<String> = sqlx::query_scalar(
        "SELECT status FROM friend_requests WHERE from_user = ? AND to_user = ?",
    )
    .bind(me)
    .bind(other)
    .fetch_optional(&state.db)
    .await?;
    if outgoing.as_deref() == Some("pending") {
        return Ok("pending_out".into());
    }
    let incoming: Option<String> = sqlx::query_scalar(
        "SELECT status FROM friend_requests WHERE from_user = ? AND to_user = ?",
    )
    .bind(other)
    .bind(me)
    .fetch_optional(&state.db)
    .await?;
    if incoming.as_deref() == Some("pending") {
        return Ok("pending_in".into());
    }
    Ok("none".into())
}

pub async fn search(
    state: &AppState,
    me: &AuthUser,
    query: &str,
) -> Result<Vec<PublicUser>, AppError> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 20 {
        return Err(AppError::bad("请输入完整的用户名或 7 位 UID"));
    }
    let mut found: Vec<UserLite> = Vec::new();
    if let Some(user) = sqlx::query_as::<_, UserLite>(
        "SELECT id, uid, username FROM users WHERE username_lower = ? AND disabled = 0",
    )
    .bind(query.to_lowercase())
    .fetch_optional(&state.db)
    .await?
    {
        found.push(user);
    }
    if query.len() == 7 && query.bytes().all(|byte| byte.is_ascii_digit()) {
        if let Some(user) = sqlx::query_as::<_, UserLite>(
            "SELECT id, uid, username FROM users WHERE uid = ? AND disabled = 0",
        )
        .bind(query)
        .fetch_optional(&state.db)
        .await?
        {
            if !found.iter().any(|item| item.id == user.id) {
                found.push(user);
            }
        }
    }
    let mut users = Vec::with_capacity(found.len());
    for user in found {
        let relation = relation(state, me.id, user.id).await?;
        users.push(PublicUser {
            id: user.id,
            uid: user.uid,
            username: user.username,
            relation: Some(relation),
        });
    }
    Ok(users)
}

pub async fn list_friends(state: &AppState, me: &AuthUser) -> Result<Vec<PublicUser>, AppError> {
    let rows = sqlx::query_as::<_, UserLite>(
        "SELECT u.id, u.uid, u.username FROM friendships f
         JOIN users u ON u.id = CASE WHEN f.user_a = ? THEN f.user_b ELSE f.user_a END
         WHERE f.user_a = ? OR f.user_b = ?
         ORDER BY u.username_lower",
    )
    .bind(me.id)
    .bind(me.id)
    .bind(me.id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|user| PublicUser {
            id: user.id,
            uid: user.uid,
            username: user.username,
            relation: Some("friends".into()),
        })
        .collect())
}

#[derive(FromRow)]
struct RequestRow {
    id: i64,
    status: String,
    created_at: i64,
    from_id: i64,
    from_uid: String,
    from_username: String,
    to_id: i64,
    to_uid: String,
    to_username: String,
}

fn map_request(row: RequestRow) -> FriendRequestDto {
    FriendRequestDto {
        id: row.id,
        status: row.status,
        created_at: row.created_at,
        from_user: PublicUser {
            id: row.from_id,
            uid: row.from_uid,
            username: row.from_username,
            relation: None,
        },
        to_user: PublicUser {
            id: row.to_id,
            uid: row.to_uid,
            username: row.to_username,
            relation: None,
        },
    }
}

const REQUEST_SQL: &str = "SELECT r.id, r.status, r.created_at,
    fu.id AS from_id, fu.uid AS from_uid, fu.username AS from_username,
    tu.id AS to_id, tu.uid AS to_uid, tu.username AS to_username
    FROM friend_requests r
    JOIN users fu ON fu.id = r.from_user
    JOIN users tu ON tu.id = r.to_user";

pub async fn list_requests(
    state: &AppState,
    me: &AuthUser,
) -> Result<FriendRequestLists, AppError> {
    let incoming = sqlx::query_as::<_, RequestRow>(&format!(
        "{REQUEST_SQL} WHERE r.to_user = ? AND r.status = 'pending' ORDER BY r.id DESC"
    ))
    .bind(me.id)
    .fetch_all(&state.db)
    .await?;
    let outgoing = sqlx::query_as::<_, RequestRow>(&format!(
        "{REQUEST_SQL} WHERE r.from_user = ? AND r.status = 'pending' ORDER BY r.id DESC"
    ))
    .bind(me.id)
    .fetch_all(&state.db)
    .await?;
    Ok(FriendRequestLists {
        incoming: incoming.into_iter().map(map_request).collect(),
        outgoing: outgoing.into_iter().map(map_request).collect(),
    })
}

pub async fn send_request(
    state: &AppState,
    me: &AuthUser,
    to_user: i64,
) -> Result<FriendRequestDto, AppError> {
    if to_user == me.id {
        return Err(AppError::bad("不能添加自己"));
    }
    let target = sqlx::query_as::<_, UserLite>(
        "SELECT id, uid, username FROM users WHERE id = ? AND disabled = 0",
    )
    .bind(to_user)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if are_friends(state, me.id, to_user).await? {
        return Err(AppError::conflict("你们已经是好友"));
    }
    let reverse: Option<String> = sqlx::query_scalar(
        "SELECT status FROM friend_requests WHERE from_user = ? AND to_user = ?",
    )
    .bind(to_user)
    .bind(me.id)
    .fetch_optional(&state.db)
    .await?;
    if reverse.as_deref() == Some("pending") {
        return Err(AppError::conflict("对方已向你发送申请，请直接处理"));
    }
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT status FROM friend_requests WHERE from_user = ? AND to_user = ?",
    )
    .bind(me.id)
    .bind(to_user)
    .fetch_optional(&state.db)
    .await?;
    if existing.as_deref() == Some("pending") {
        return Err(AppError::conflict("申请已发送"));
    }
    let now = now_ms();
    if existing.is_some() {
        sqlx::query(
            "UPDATE friend_requests SET status = 'pending', updated_at = ? WHERE from_user = ? AND to_user = ?",
        )
        .bind(now)
        .bind(me.id)
        .bind(to_user)
        .execute(&state.db)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO friend_requests (from_user, to_user, status, created_at, updated_at) VALUES (?, ?, 'pending', ?, ?)",
        )
        .bind(me.id)
        .bind(to_user)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await?;
    }
    let row = sqlx::query_as::<_, RequestRow>(&format!(
        "{REQUEST_SQL} WHERE r.from_user = ? AND r.to_user = ?"
    ))
    .bind(me.id)
    .bind(target.id)
    .fetch_one(&state.db)
    .await?;
    let dto = map_request(row);
    domain::emit(
        state,
        &[to_user],
        &ServerEvent::FriendRequest {
            request: dto.clone(),
        },
    );
    Ok(dto)
}

pub async fn accept(state: &AppState, me: &AuthUser, request_id: i64) -> Result<(), AppError> {
    let mut tx = state.db.begin().await?;
    #[derive(FromRow)]
    struct Pair {
        from_user: i64,
        to_user: i64,
    }
    let pair = sqlx::query_as::<_, Pair>(
        "SELECT from_user, to_user FROM friend_requests WHERE id = ? AND to_user = ? AND status = 'pending'",
    )
    .bind(request_id)
    .bind(me.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let now = now_ms();
    sqlx::query("UPDATE friend_requests SET status = 'accepted', updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(request_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE friend_requests SET status = 'accepted', updated_at = ? WHERE from_user = ? AND to_user = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(pair.to_user)
    .bind(pair.from_user)
    .execute(&mut *tx)
    .await?;
    let (left, right) = ordered(pair.from_user, pair.to_user);
    sqlx::query(
        "INSERT INTO friendships (user_a, user_b, created_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(left)
    .bind(right)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let me_user = public_user(state, me.id).await?;
    let other = public_user(state, pair.from_user).await?;
    domain::emit(
        state,
        &[pair.from_user],
        &ServerEvent::Friendship {
            user: me_user,
            status: "friends".into(),
        },
    );
    domain::emit(
        state,
        &[me.id],
        &ServerEvent::Friendship {
            user: other,
            status: "friends".into(),
        },
    );
    Ok(())
}

pub async fn reject(state: &AppState, me: &AuthUser, request_id: i64) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE friend_requests SET status = 'rejected', updated_at = ? WHERE id = ? AND to_user = ? AND status = 'pending'",
    )
    .bind(now_ms())
    .bind(request_id)
    .bind(me.id)
    .execute(&state.db)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub async fn remove_friend(state: &AppState, me: &AuthUser, other_id: i64) -> Result<(), AppError> {
    let (left, right) = ordered(me.id, other_id);
    let changed = sqlx::query("DELETE FROM friendships WHERE user_a = ? AND user_b = ?")
        .bind(left)
        .bind(right)
        .execute(&state.db)
        .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query(
        "DELETE FROM friend_requests WHERE (from_user = ? AND to_user = ?) OR (from_user = ? AND to_user = ?)",
    )
    .bind(me.id)
    .bind(other_id)
    .bind(other_id)
    .bind(me.id)
    .execute(&state.db)
    .await?;
    let me_user = public_user(state, me.id).await?;
    let other = public_user(state, other_id).await?;
    domain::emit(
        state,
        &[other_id],
        &ServerEvent::Friendship {
            user: me_user,
            status: "none".into(),
        },
    );
    domain::emit(
        state,
        &[me.id],
        &ServerEvent::Friendship {
            user: other,
            status: "none".into(),
        },
    );
    Ok(())
}

pub async fn public_user(state: &AppState, id: i64) -> Result<PublicUser, AppError> {
    let user = sqlx::query_as::<_, UserLite>("SELECT id, uid, username FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(PublicUser {
        id: user.id,
        uid: user.uid,
        username: user.username,
        relation: None,
    })
}
