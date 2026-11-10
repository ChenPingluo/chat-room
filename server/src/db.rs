use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, SqlitePool};

use crate::error::AppError;

pub async fn connect(path: &Path) -> Result<SqlitePool, AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::internal)?;
    }
    let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
        .map_err(|err| AppError::internal(anyhow::anyhow!(err)))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
        .pragma("cache_size", "-8000")
        .pragma("temp_store", "MEMORY")
        .pragma("wal_autocheckpoint", "1000")
        .disable_statement_logging();
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    // 必须在建表之前设置，之后只能通过 VACUUM 改变，所以每次启动都试一次。
    // 赋值形式的 PRAGMA 不一定返回行。
    let _: Vec<(i64,)> = sqlx::query_as("PRAGMA auto_vacuum = 2")
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|err| AppError::internal(anyhow::anyhow!(err)))?;
    Ok(pool)
}
