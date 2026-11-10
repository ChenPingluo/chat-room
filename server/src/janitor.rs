use std::path::Path;

use sqlx::FromRow;

use crate::domain::media;
use crate::error::AppError;
use crate::state::AppState;
use crate::util::now_ms;

#[derive(Debug, Default)]
pub struct CleanupStats {
    pub messages_deleted: u64,
    pub attachments_deleted: u64,
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(err) = cleanup(&state).await {
                tracing::error!(error = %err, "定时清理失败");
            }
        }
    });
}

pub async fn cleanup(state: &AppState) -> Result<CleanupStats, AppError> {
    let now = now_ms();
    let mut stats = CleanupStats::default();
    #[derive(FromRow)]
    struct FileRow {
        id: i64,
        file_name: String,
        thumb_file_name: String,
    }
    let orphan_before = now - 60 * 60 * 1000;
    let files = sqlx::query_as::<_, FileRow>(
        "SELECT a.id, a.file_name, a.thumb_file_name
         FROM attachments a
         LEFT JOIN messages m ON m.id = a.message_id
         WHERE (a.message_id IS NOT NULL AND m.expires_at <= ?)
            OR (a.message_id IS NULL AND a.created_at <= ?)
            OR (m.deleted_at IS NOT NULL)",
    )
    .bind(now)
    .bind(orphan_before)
    .fetch_all(&state.db)
    .await?;
    let dir = state.files_dir();
    for file in &files {
        sqlx::query("UPDATE messages SET attachment_id = NULL WHERE attachment_id = ?")
            .bind(file.id)
            .execute(&state.db)
            .await?;
        sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(file.id)
            .execute(&state.db)
            .await?;
        media::remove_files(&dir, &file.file_name, &file.thumb_file_name);
        stats.attachments_deleted += 1;
    }
    let deleted = sqlx::query("DELETE FROM messages WHERE expires_at <= ?")
        .bind(now)
        .execute(&state.db)
        .await?;
    stats.messages_deleted = deleted.rows_affected();
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now)
        .execute(&state.db)
        .await?;
    let old = now - 24 * 60 * 60 * 1000;
    sqlx::query("DELETE FROM login_attempts WHERE updated_at < ? AND locked_until < ?")
        .bind(old)
        .bind(now)
        .execute(&state.db)
        .await?;
    let invite_cutoff = now - 30 * 24 * 60 * 60 * 1000;
    sqlx::query(
        "DELETE FROM invites WHERE used_by IS NULL AND (revoked = 1 OR expires_at < ?) AND created_at < ?",
    )
    .bind(invite_cutoff)
    .bind(invite_cutoff)
    .execute(&state.db)
    .await?;
    maybe_snapshot(state, now).await;
    Ok(stats)
}

async fn maybe_snapshot(state: &AppState, now: i64) {
    let last: Option<String> =
        match sqlx::query_scalar("SELECT value FROM meta WHERE key = 'last_snapshot'")
            .fetch_optional(&state.db)
            .await
        {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(error = %err, "读取备份时间失败");
                return;
            }
        };
    let due = last
        .and_then(|value| value.parse::<i64>().ok())
        .map(|stamp| now - stamp >= 24 * 60 * 60 * 1000)
        .unwrap_or(true);
    if !due {
        return;
    }
    let dir = state.config.data_dir.join("backups");
    if let Err(err) = snapshot_database(&state.db, &dir).await {
        tracing::warn!(error = %err, "数据库快照失败");
        return;
    }
    let _ = sqlx::query(
        "INSERT INTO meta (key, value) VALUES ('last_snapshot', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(now.to_string())
    .execute(&state.db)
    .await;
}

pub async fn snapshot_database(db: &sqlx::SqlitePool, dir: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(dir).map_err(AppError::internal)?;
    let dest = dir.join("chat.db");
    if dest.exists() {
        std::fs::remove_file(&dest).map_err(AppError::internal)?;
    }
    let text = dest
        .to_str()
        .ok_or_else(|| AppError::bad("备份路径不是 UTF-8"))?
        .replace('\'', "''");
    if text.contains('\n') {
        return Err(AppError::bad("备份路径无效"));
    }
    sqlx::query(&format!("VACUUM INTO '{text}'"))
        .execute(db)
        .await?;
    Ok(())
}
