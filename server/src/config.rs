use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::util::decode_master_key;

#[derive(Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub public_base_url: String,
    pub allowed_origin: String,
    pub master_key: [u8; 32],
    pub cookie_secure: bool,
}

impl Config {
    pub fn for_test(data_dir: &Path) -> Self {
        Self {
            listen: "127.0.0.1:0".parse().expect("地址"),
            data_dir: data_dir.to_path_buf(),
            public_base_url: "http://127.0.0.1:8080".to_string(),
            allowed_origin: "http://127.0.0.1:8080".to_string(),
            master_key: [7u8; 32],
            cookie_secure: false,
        }
    }

    pub fn from_env() -> Result<Self, AppError> {
        let data_dir = std::env::var("CHAT_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./data"));
        let listen = std::env::var("CHAT_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .map_err(|_| AppError::bad("CHAT_LISTEN 不是有效的地址"))?;
        let public_base_url =
            std::env::var("CHAT_PUBLIC_BASE_URL").unwrap_or_else(|_| format!("http://{listen}"));
        let allowed_origin = parse_origin(&public_base_url)?;
        let master_key = load_master_key()?;
        let cookie_secure = match std::env::var("CHAT_COOKIE_SECURE") {
            Ok(value) => matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"),
            Err(_) => public_base_url.starts_with("https://"),
        };
        Ok(Self {
            listen,
            data_dir,
            public_base_url: public_base_url.trim_end_matches('/').to_string(),
            allowed_origin,
            master_key,
            cookie_secure,
        })
    }
}

impl Config {
    pub fn with_overrides(
        mut self,
        data_dir: Option<PathBuf>,
        listen: Option<SocketAddr>,
        public_base_url: Option<String>,
    ) -> Result<Self, AppError> {
        if let Some(dir) = data_dir {
            self.data_dir = dir;
        }
        if let Some(listen) = listen {
            self.listen = listen;
        }
        if let Some(url) = public_base_url {
            self.allowed_origin = parse_origin(&url)?;
            self.public_base_url = url.trim_end_matches('/').to_string();
            if std::env::var("CHAT_COOKIE_SECURE").is_err() {
                self.cookie_secure = self.public_base_url.starts_with("https://");
            }
        }
        Ok(self)
    }
}

fn parse_origin(base: &str) -> Result<String, AppError> {
    let rest = base.trim().trim_end_matches('/');
    let Some((scheme, after)) = rest.split_once("://") else {
        return Err(AppError::bad(
            "CHAT_PUBLIC_BASE_URL 需要包含 http:// 或 https://",
        ));
    };
    if scheme != "http" && scheme != "https" {
        return Err(AppError::bad("CHAT_PUBLIC_BASE_URL 只支持 http 或 https"));
    }
    let (authority, path) = after
        .split_once('/')
        .map(|(authority, path)| (authority, Some(path)))
        .unwrap_or((after, None));
    if authority.is_empty() || authority.contains(' ') {
        return Err(AppError::bad("CHAT_PUBLIC_BASE_URL 的主机名无效"));
    }
    if path.is_some_and(|path| !path.is_empty()) {
        return Err(AppError::bad(
            "CHAT_PUBLIC_BASE_URL 不能包含路径，应用挂在网站根路径",
        ));
    }
    Ok(format!("{scheme}://{authority}"))
}

fn load_master_key() -> Result<[u8; 32], AppError> {
    if let Ok(path) = std::env::var("CHAT_MASTER_KEY_FILE") {
        let bytes = std::fs::read(&path).map_err(|err| {
            AppError::internal(anyhow::anyhow!("无法读取主密钥文件 {path}: {err}"))
        })?;
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| AppError::bad("主密钥文件不是文本，也不是 32 字节原始密钥"))?;
        return decode_master_key(&text).map_err(AppError::bad);
    }
    if let Ok(text) = std::env::var("CHAT_MASTER_KEY") {
        return decode_master_key(&text).map_err(AppError::bad);
    }
    Err(AppError::bad(
        "缺少主密钥。请设置 CHAT_MASTER_KEY_FILE，或运行 chat-server keygen 生成一个",
    ))
}
