use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::Semaphore;

use crate::config::Config;
use crate::crypto::Crypto;
use crate::db;
use crate::error::AppError;
use crate::hub::Hub;
use crate::password;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub crypto: Crypto,
    pub hub: Arc<Hub>,
    pub argon: Arc<Semaphore>,
    pub images: Arc<Semaphore>,
    pub dummy_hash: Arc<String>,
}

impl AppState {
    pub async fn boot(config: Config) -> Result<Self, AppError> {
        std::fs::create_dir_all(&config.data_dir).map_err(AppError::internal)?;
        std::fs::set_permissions(&config.data_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(AppError::internal)?;
        let files = config.data_dir.join("files");
        std::fs::create_dir_all(&files).map_err(AppError::internal)?;
        std::fs::set_permissions(&files, std::fs::Permissions::from_mode(0o700))
            .map_err(AppError::internal)?;
        let db = db::connect(&config.data_dir.join("chat.db")).await?;
        let crypto = Crypto::new(config.master_key);
        let dummy_hash =
            password::hash_password(&format!("dummy-{}", crate::util::random_token()?))?;
        Ok(Self {
            db,
            config: Arc::new(config),
            crypto,
            hub: Arc::new(Hub::new()),
            argon: Arc::new(Semaphore::new(2)),
            images: Arc::new(Semaphore::new(1)),
            dummy_hash: Arc::new(dummy_hash),
        })
    }

    pub fn files_dir(&self) -> std::path::PathBuf {
        self.config.data_dir.join("files")
    }
}
