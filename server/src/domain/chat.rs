use sqlx::FromRow;

use crate::auth::AuthUser;
use crate::domain::{self, media, social};
use crate::dto::{AttachmentDto, ConversationDto, MemberDto, MessageDto, ServerEvent};
use crate::error::AppError;
use crate::state::AppState;
use crate::util::{self, now_ms, validate_group_name, DEFAULT_RETENTION_SECS, MAX_GROUP_MEMBERS};

#[derive(FromRow)]
struct ConvRow {
    id: i64,
    kind: String,
    name: Option<String>,
    owner_id: Option<i64>,
    retention_secs: i64,
    created_at: i64,
    role: String,
    last_read_id: i64,
}

#[derive(FromRow)]
struct MessageRow {
    id: i64,
    conv_id: i64,
    sender_id: i64,
    kind: String,
    body_ct: Option<Vec<u8>>,
    nonce: Option<Vec<u8>>,
    attachment_id: Option<i64>,
    created_at: i64,
    expires_at: i64,
    deleted_at: Option<i64>,
}

#[derive(FromRow)]
struct MemberRow {
    id: i64,
    uid: String,
    username: String,
    role: String,
}

pub async fn list_conversations(
    state: &AppState,
    user_id: i64,
) -> Result<Vec<ConversationDto>, AppError> {
    let rows = sqlx::query_as::<_, ConvRow>(
        "SELECT c.id, c.kind, c.name, c.owner_id, c.retention_secs, c.created_at, m.role, m.last_read_id
         FROM conversations c
         JOIN conversation_members m ON m.conv_id = c.id
         WHERE m.user_id = ?
         ORDER BY c.id DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(hydrate(state, user_id, row).await?);
    }
    out.sort_by(|left, right| {
        let left_at = left
            .last_message
            .as_ref()
            .map(|msg| msg.created_at)
            .unwrap_or(left.created_at);
        let right_at = right
            .last_message
            .as_ref()
            .map(|msg| msg.created_at)
            .unwrap_or(right.created_at);
        right_at.cmp(&left_at)
    });
    Ok(out)
}

pub async fn get_conversation(
    state: &AppState,
    user_id: i64,
    conv_id: i64,
) -> Result<ConversationDto, AppError> {
    let row = load_member_conv(state, user_id, conv_id).await?;
    hydrate(state, user_id, row).await
}

async fn load_member_conv(
    state: &AppState,
    user_id: i64,
    conv_id: i64,
) -> Result<ConvRow, AppError> {
    sqlx::query_as::<_, ConvRow>(
        "SELECT c.id, c.kind, c.name, c.owner_id, c.retention_secs, c.created_at, m.role, m.last_read_id
         FROM conversations c
         JOIN conversation_members m ON m.conv_id = c.id
         WHERE c.id = ? AND m.user_id = ?",
    )
    .bind(conv_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

async fn hydrate(
    state: &AppState,
    user_id: i64,
    row: ConvRow,
) -> Result<ConversationDto, AppError> {
    let members = members_of(state, row.id).await?;
    let last = latest_message(state, row.id).await?;
    let unread: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE conv_id = ? AND id > ? AND sender_id != ? AND deleted_at IS NULL",
    )
    .bind(row.id)
    .bind(row.last_read_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    Ok(ConversationDto {
        id: row.id,
        kind: row.kind,
        name: row.name,
        retention_secs: row.retention_secs,
        my_role: row.role,
        owner_id: row.owner_id,
        members,
        last_message: last,
        unread,
        created_at: row.created_at,
    })
}

async fn members_of(state: &AppState, conv_id: i64) -> Result<Vec<MemberDto>, AppError> {
    let rows = sqlx::query_as::<_, MemberRow>(
        "SELECT u.id, u.uid, u.username, m.role
         FROM conversation_members m JOIN users u ON u.id = m.user_id
         WHERE m.conv_id = ? ORDER BY m.joined_at, u.id",
    )
    .bind(conv_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| MemberDto {
            id: row.id,
            uid: row.uid,
            username: row.username,
            role: row.role,
        })
        .collect())
}

async fn member_ids(state: &AppState, conv_id: i64) -> Result<Vec<i64>, AppError> {
    Ok(
        sqlx::query_scalar("SELECT user_id FROM conversation_members WHERE conv_id = ?")
            .bind(conv_id)
            .fetch_all(&state.db)
            .await?,
    )
}

pub async fn create_direct(
    state: &AppState,
    me: &AuthUser,
    other_id: i64,
) -> Result<ConversationDto, AppError> {
    if other_id == me.id {
        return Err(AppError::bad("不能和自己私聊"));
    }
    if !social::are_friends(state, me.id, other_id).await? {
        return Err(AppError::forbidden("成为好友之后才能私聊"));
    }
    let (left, right) = if me.id < other_id {
        (me.id, other_id)
    } else {
        (other_id, me.id)
    };
    let key = format!("{left}:{right}");
    if let Some(id) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM conversations WHERE direct_key = ?")
            .bind(&key)
            .fetch_optional(&state.db)
            .await?
    {
        return get_conversation(state, me.id, id).await;
    }
    let now = now_ms();
    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversations (kind, name, owner_id, retention_secs, direct_key, created_at)
         VALUES ('direct', NULL, NULL, ?, ?, ?) RETURNING id",
    )
    .bind(DEFAULT_RETENTION_SECS)
    .bind(&key)
    .bind(now)
    .fetch_one(&mut *tx)
    .await;
    let id = match inserted {
        Ok(id) => id,
        Err(sqlx::Error::Database(err)) if err.message().contains("direct_key") => {
            drop(tx);
            let id: i64 = sqlx::query_scalar("SELECT id FROM conversations WHERE direct_key = ?")
                .bind(&key)
                .fetch_one(&state.db)
                .await?;
            return get_conversation(state, me.id, id).await;
        }
        Err(err) => return Err(err.into()),
    };
    for user_id in [left, right] {
        sqlx::query(
            "INSERT INTO conversation_members (conv_id, user_id, role, joined_at, last_read_id) VALUES (?, ?, 'member', ?, 0)",
        )
        .bind(id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let dto = get_conversation(state, me.id, id).await?;
    broadcast_conversation(state, id).await?;
    Ok(dto)
}

pub async fn create_group(
    state: &AppState,
    me: &AuthUser,
    name: &str,
    member_ids_in: &[i64],
) -> Result<ConversationDto, AppError> {
    let name = validate_group_name(name)?;
    let mut ids: Vec<i64> = member_ids_in
        .iter()
        .copied()
        .filter(|id| *id != me.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() as i64 + 1 > MAX_GROUP_MEMBERS {
        return Err(AppError::bad("群成员不能超过 100 人"));
    }
    for id in &ids {
        if !social::are_friends(state, me.id, *id).await? {
            return Err(AppError::forbidden("只能把好友拉进群"));
        }
    }
    let now = now_ms();
    let mut tx = state.db.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO conversations (kind, name, owner_id, retention_secs, direct_key, created_at)
         VALUES ('group', ?, ?, ?, NULL, ?) RETURNING id",
    )
    .bind(&name)
    .bind(me.id)
    .bind(DEFAULT_RETENTION_SECS)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO conversation_members (conv_id, user_id, role, joined_at, last_read_id) VALUES (?, ?, 'owner', ?, 0)",
    )
    .bind(id)
    .bind(me.id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    for user_id in &ids {
        sqlx::query(
            "INSERT INTO conversation_members (conv_id, user_id, role, joined_at, last_read_id) VALUES (?, ?, 'member', ?, 0)",
        )
        .bind(id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    broadcast_conversation(state, id).await?;
    get_conversation(state, me.id, id).await
}

pub async fn patch_conversation(
    state: &AppState,
    me: &AuthUser,
    conv_id: i64,
    name: Option<&str>,
    retention_secs: Option<i64>,
) -> Result<ConversationDto, AppError> {
    let row = load_member_conv(state, me.id, conv_id).await?;
    if let Some(retention) = retention_secs {
        if !util::valid_retention(retention) {
            return Err(AppError::bad("保留时间只能是 1 小时、1 天、7 天或 30 天"));
        }
        if row.kind == "group" && row.role != "owner" {
            return Err(AppError::forbidden("只有群主可以修改保留时间"));
        }
        sqlx::query("UPDATE conversations SET retention_secs = ? WHERE id = ?")
            .bind(retention)
            .bind(conv_id)
            .execute(&state.db)
            .await?;
    }
    if let Some(name) = name {
        if row.kind != "group" {
            return Err(AppError::bad("私聊没有群名"));
        }
        if row.role != "owner" {
            return Err(AppError::forbidden("只有群主可以修改群名"));
        }
        let name = validate_group_name(name)?;
        sqlx::query("UPDATE conversations SET name = ? WHERE id = ?")
            .bind(name)
            .bind(conv_id)
            .execute(&state.db)
            .await?;
    }
    broadcast_conversation(state, conv_id).await?;
    get_conversation(state, me.id, conv_id).await
}

pub async fn add_member(
    state: &AppState,
    me: &AuthUser,
    conv_id: i64,
    user_id: i64,
) -> Result<ConversationDto, AppError> {
    let row = load_member_conv(state, me.id, conv_id).await?;
    if row.kind != "group" {
        return Err(AppError::bad("私聊不能再加人"));
    }
    if row.role != "owner" {
        return Err(AppError::forbidden("只有群主可以拉人"));
    }
    if !social::are_friends(state, me.id, user_id).await? {
        return Err(AppError::forbidden("只能把好友拉进群"));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversation_members WHERE conv_id = ?")
            .bind(conv_id)
            .fetch_one(&state.db)
            .await?;
    if count >= MAX_GROUP_MEMBERS {
        return Err(AppError::bad("群成员不能超过 100 人"));
    }
    let now = now_ms();
    let inserted = sqlx::query(
        "INSERT INTO conversation_members (conv_id, user_id, role, joined_at, last_read_id) VALUES (?, ?, 'member', ?, 0)",
    )
    .bind(conv_id)
    .bind(user_id)
    .bind(now)
    .execute(&state.db)
    .await;
    if let Err(sqlx::Error::Database(err)) = &inserted {
        if err.message().contains("PRIMARY") || err.message().contains("constraint") {
            return Err(AppError::conflict("对方已经在群里"));
        }
    }
    inserted?;
    broadcast_conversation(state, conv_id).await?;
    get_conversation(state, me.id, conv_id).await
}

pub async fn kick_member(
    state: &AppState,
    me: &AuthUser,
    conv_id: i64,
    user_id: i64,
) -> Result<(), AppError> {
    let row = load_member_conv(state, me.id, conv_id).await?;
    if row.kind != "group" {
        return Err(AppError::bad("不能把私聊成员移出"));
    }
    if row.role != "owner" {
        return Err(AppError::forbidden("只有群主可以移出成员"));
    }
    if user_id == me.id {
        return Err(AppError::bad("请使用退出，或先转让群主"));
    }
    let changed = sqlx::query(
        "DELETE FROM conversation_members WHERE conv_id = ? AND user_id = ? AND role != 'owner'",
    )
    .bind(conv_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    broadcast_conversation(state, conv_id).await?;
    state.hub.emit(&[user_id], removed_event(conv_id));
    Ok(())
}

fn removed_event(conv_id: i64) -> String {
    serde_json::json!({ "type": "conversation_removed", "conv_id": conv_id }).to_string()
}

pub async fn leave(state: &AppState, me: &AuthUser, conv_id: i64) -> Result<(), AppError> {
    let row = load_member_conv(state, me.id, conv_id).await?;
    if row.kind != "group" {
        return Err(AppError::bad("私聊不能退出，可以删除好友"));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversation_members WHERE conv_id = ?")
            .bind(conv_id)
            .fetch_one(&state.db)
            .await?;
    if row.role == "owner" && count > 1 {
        return Err(AppError::bad("请先转让群主"));
    }
    if count <= 1 {
        destroy_conversation(state, conv_id).await?;
        state.hub.emit(&[me.id], removed_event(conv_id));
        return Ok(());
    }
    sqlx::query("DELETE FROM conversation_members WHERE conv_id = ? AND user_id = ?")
        .bind(conv_id)
        .bind(me.id)
        .execute(&state.db)
        .await?;
    broadcast_conversation(state, conv_id).await?;
    state.hub.emit(&[me.id], removed_event(conv_id));
    Ok(())
}

pub async fn transfer(
    state: &AppState,
    me: &AuthUser,
    conv_id: i64,
    user_id: i64,
) -> Result<ConversationDto, AppError> {
    let row = load_member_conv(state, me.id, conv_id).await?;
    if row.kind != "group" || row.role != "owner" {
        return Err(AppError::forbidden("只有群主可以转让"));
    }
    if user_id == me.id {
        return Err(AppError::bad("对方已经是群主"));
    }
    let target: Option<String> = sqlx::query_scalar(
        "SELECT role FROM conversation_members WHERE conv_id = ? AND user_id = ?",
    )
    .bind(conv_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    if target.is_none() {
        return Err(AppError::bad("对方不在群里"));
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE conversation_members SET role = 'member' WHERE conv_id = ? AND user_id = ?",
    )
    .bind(conv_id)
    .bind(me.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE conversation_members SET role = 'owner' WHERE conv_id = ? AND user_id = ?")
        .bind(conv_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE conversations SET owner_id = ? WHERE id = ?")
        .bind(user_id)
        .bind(conv_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    broadcast_conversation(state, conv_id).await?;
    get_conversation(state, me.id, conv_id).await
}

async fn destroy_conversation(state: &AppState, conv_id: i64) -> Result<(), AppError> {
    #[derive(FromRow)]
    struct FileRow {
        id: i64,
        file_name: String,
        thumb_file_name: String,
    }
    let files = sqlx::query_as::<_, FileRow>(
        "SELECT a.id, a.file_name, a.thumb_file_name FROM attachments a
         JOIN messages m ON m.attachment_id = a.id WHERE m.conv_id = ?",
    )
    .bind(conv_id)
    .fetch_all(&state.db)
    .await?;
    let ids: Vec<i64> = files.iter().map(|file| file.id).collect();
    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(conv_id)
        .execute(&state.db)
        .await?;
    for id in ids {
        sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    let dir = state.files_dir();
    for file in files {
        media::remove_files(&dir, &file.file_name, &file.thumb_file_name);
    }
    Ok(())
}

pub async fn mark_read(
    state: &AppState,
    user_id: i64,
    conv_id: i64,
    message_id: i64,
) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE conversation_members
         SET last_read_id = CASE WHEN last_read_id < ? THEN ? ELSE last_read_id END
         WHERE conv_id = ? AND user_id = ?",
    )
    .bind(message_id)
    .bind(message_id)
    .bind(conv_id)
    .bind(user_id)
    .execute(&state.db)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub async fn list_messages(
    state: &AppState,
    user_id: i64,
    conv_id: i64,
    before: Option<i64>,
    after: Option<i64>,
    limit: i64,
) -> Result<Vec<MessageDto>, AppError> {
    let _ = load_member_conv(state, user_id, conv_id).await?;
    let limit = limit.clamp(1, 100);
    let rows = if let Some(before) = before {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conv_id, sender_id, kind, body_ct, nonce, attachment_id, created_at, expires_at, deleted_at
             FROM messages WHERE conv_id = ? AND id < ? ORDER BY id DESC LIMIT ?",
        )
        .bind(conv_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    } else if let Some(after) = after {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conv_id, sender_id, kind, body_ct, nonce, attachment_id, created_at, expires_at, deleted_at
             FROM messages WHERE conv_id = ? AND id > ? ORDER BY id ASC LIMIT ?",
        )
        .bind(conv_id)
        .bind(after)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conv_id, sender_id, kind, body_ct, nonce, attachment_id, created_at, expires_at, deleted_at
             FROM messages WHERE conv_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(conv_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    };
    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        messages.push(message_from_row(state, row, None).await?);
    }
    if before.is_some() || after.is_none() {
        messages.reverse();
    }
    Ok(messages)
}

async fn latest_message(state: &AppState, conv_id: i64) -> Result<Option<MessageDto>, AppError> {
    let row = sqlx::query_as::<_, MessageRow>(
        "SELECT id, conv_id, sender_id, kind, body_ct, nonce, attachment_id, created_at, expires_at, deleted_at
         FROM messages WHERE conv_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(conv_id)
    .fetch_optional(&state.db)
    .await?;
    match row {
        Some(row) => Ok(Some(message_from_row(state, row, None).await?)),
        None => Ok(None),
    }
}

async fn message_from_row(
    state: &AppState,
    row: MessageRow,
    client_msg_id: Option<String>,
) -> Result<MessageDto, AppError> {
    if row.deleted_at.is_some() {
        return Ok(MessageDto {
            id: row.id,
            conv_id: row.conv_id,
            sender_id: row.sender_id,
            kind: row.kind,
            body: None,
            attachment: None,
            created_at: row.created_at,
            expires_at: row.expires_at,
            deleted: true,
            client_msg_id,
        });
    }
    let body = match (&row.body_ct, &row.nonce) {
        (Some(ct), Some(nonce)) => {
            let plain = state.crypto.open(nonce, ct)?;
            Some(String::from_utf8_lossy(&plain).into_owned())
        }
        _ => None,
    };
    let attachment = if let Some(id) = row.attachment_id {
        #[derive(FromRow)]
        struct Meta {
            width: i64,
            height: i64,
            size: i64,
        }
        let meta =
            sqlx::query_as::<_, Meta>("SELECT width, height, size FROM attachments WHERE id = ?")
                .bind(id)
                .fetch_optional(&state.db)
                .await?;
        meta.map(|meta| AttachmentDto {
            id,
            width: meta.width,
            height: meta.height,
            size: meta.size,
        })
    } else {
        None
    };
    Ok(MessageDto {
        id: row.id,
        conv_id: row.conv_id,
        sender_id: row.sender_id,
        kind: row.kind,
        body,
        attachment,
        created_at: row.created_at,
        expires_at: row.expires_at,
        deleted: false,
        client_msg_id,
    })
}

pub async fn send_message(
    state: &AppState,
    me: &AuthUser,
    conv_id: i64,
    kind: &str,
    body: Option<&str>,
    attachment_id: Option<i64>,
) -> Result<MessageDto, AppError> {
    let row = load_member_conv(state, me.id, conv_id).await?;
    if row.kind == "direct" {
        let peer: Option<i64> = sqlx::query_scalar(
            "SELECT user_id FROM conversation_members WHERE conv_id = ? AND user_id != ?",
        )
        .bind(conv_id)
        .bind(me.id)
        .fetch_optional(&state.db)
        .await?;
        let Some(peer) = peer else {
            return Err(AppError::bad("私聊缺少对方"));
        };
        if !social::are_friends(state, me.id, peer).await? {
            return Err(AppError::forbidden("你们还不是好友"));
        }
    }
    let now = now_ms();
    let expires_at = now + row.retention_secs.min(util::MAX_RETENTION_SECS);
    let (body_ct, nonce, attachment) = match kind {
        "text" => {
            let text = body.unwrap_or("");
            util::validate_text(text)?;
            let (ct, nonce) = state.crypto.seal(text.as_bytes())?;
            (Some(ct), Some(nonce), None)
        }
        "image" => {
            let attachment_id = attachment_id.ok_or_else(|| AppError::bad("缺少图片"))?;
            let (ct, nonce) = if let Some(text) = body.filter(|text| !text.trim().is_empty()) {
                util::validate_text(text)?;
                let (ct, nonce) = state.crypto.seal(text.as_bytes())?;
                (Some(ct), Some(nonce))
            } else {
                (None, None)
            };
            (ct, nonce, Some(attachment_id))
        }
        _ => return Err(AppError::bad("不支持的消息类型")),
    };
    let mut tx = state.db.begin().await?;
    if let Some(attachment_id) = attachment {
        #[derive(FromRow)]
        struct Own {
            uploader_id: i64,
            message_id: Option<i64>,
        }
        let own = sqlx::query_as::<_, Own>(
            "SELECT uploader_id, message_id FROM attachments WHERE id = ?",
        )
        .bind(attachment_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::bad("图片不存在"))?;
        if own.uploader_id != me.id || own.message_id.is_some() {
            return Err(AppError::forbidden("不能使用这张图片"));
        }
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO messages (conv_id, sender_id, kind, body_ct, nonce, attachment_id, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(conv_id)
    .bind(me.id)
    .bind(kind)
    .bind(&body_ct)
    .bind(&nonce)
    .bind(attachment)
    .bind(now)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(attachment_id) = attachment {
        let claimed = sqlx::query(
            "UPDATE attachments SET message_id = ? WHERE id = ? AND message_id IS NULL AND uploader_id = ?",
        )
        .bind(id)
        .bind(attachment_id)
        .bind(me.id)
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() != 1 {
            return Err(AppError::conflict("图片已经被使用"));
        }
    }
    sqlx::query(
        "UPDATE conversation_members SET last_read_id = CASE WHEN last_read_id < ? THEN ? ELSE last_read_id END WHERE conv_id = ? AND user_id = ?",
    )
    .bind(id)
    .bind(id)
    .bind(conv_id)
    .bind(me.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let stored = sqlx::query_as::<_, MessageRow>(
        "SELECT id, conv_id, sender_id, kind, body_ct, nonce, attachment_id, created_at, expires_at, deleted_at FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    let dto = message_from_row(state, stored, None).await?;
    let ids = member_ids(state, conv_id).await?;
    domain::emit(
        state,
        &ids,
        &ServerEvent::Message {
            message: dto.clone(),
        },
    );
    Ok(dto)
}

pub async fn delete_message(
    state: &AppState,
    me: &AuthUser,
    conv_id: i64,
    message_id: i64,
) -> Result<(), AppError> {
    let _ = load_member_conv(state, me.id, conv_id).await?;
    #[derive(FromRow)]
    struct OwnMsg {
        sender_id: i64,
        attachment_id: Option<i64>,
        deleted_at: Option<i64>,
    }
    let message = sqlx::query_as::<_, OwnMsg>(
        "SELECT sender_id, attachment_id, deleted_at FROM messages WHERE id = ? AND conv_id = ?",
    )
    .bind(message_id)
    .bind(conv_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if message.sender_id != me.id {
        return Err(AppError::forbidden("只能删除自己的消息"));
    }
    if message.deleted_at.is_some() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE messages SET deleted_at = ?, body_ct = NULL, nonce = NULL, attachment_id = NULL WHERE id = ?",
    )
    .bind(now_ms())
    .bind(message_id)
    .execute(&state.db)
    .await?;
    if let Some(attachment_id) = message.attachment_id {
        media::delete_attachment(state, attachment_id).await?;
    }
    let ids = member_ids(state, conv_id).await?;
    domain::emit(
        state,
        &ids,
        &ServerEvent::MessageDeleted {
            conv_id,
            message_id,
        },
    );
    Ok(())
}

async fn broadcast_conversation(state: &AppState, conv_id: i64) -> Result<(), AppError> {
    let ids = member_ids(state, conv_id).await?;
    for user_id in &ids {
        if let Ok(dto) = get_conversation(state, *user_id, conv_id).await {
            domain::emit(
                state,
                &[*user_id],
                &ServerEvent::Conversation { conversation: dto },
            );
        }
    }
    Ok(())
}
