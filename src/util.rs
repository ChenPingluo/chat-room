use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf
}

pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn b64_decode(text: &str) -> Result<Vec<u8>, ()> {
    URL_SAFE_NO_PAD.decode(text).map_err(|_| ())
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

pub fn cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let mut kv = part.trim().splitn(2, '=');
        if kv.next()?.trim() == name {
            return Some(kv.next().unwrap_or("").trim().to_string());
        }
    }
    None
}

pub fn clean_username(input: &str) -> Result<String, &'static str> {
    let name = input.trim();
    if !(3..=20).contains(&name.len())
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("用户名需要 3 到 20 位字母、数字或下划线");
    }
    Ok(name.to_string())
}

pub fn clean_display(input: &str) -> Result<String, &'static str> {
    let name = input.trim();
    let count = name.chars().count();
    if !(1..=32).contains(&count) || name.chars().any(|c| c.is_control()) {
        return Err("显示名称需要 1 到 32 个字符");
    }
    Ok(name.to_string())
}

pub fn clean_password(input: &str) -> Result<&str, &'static str> {
    if !(10..=128).contains(&input.len())
        || input.chars().any(|c| c == '\0' || c == '\n' || c == '\r')
    {
        return Err("密码需要 10 到 128 个字符");
    }
    Ok(input)
}

pub fn clean_group_name(input: &str) -> Result<String, &'static str> {
    let name = input.trim();
    let count = name.chars().count();
    if !(1..=40).contains(&count) || name.chars().any(|c| c.is_control()) {
        return Err("群名称需要 1 到 40 个字符");
    }
    Ok(name.to_string())
}

pub fn clean_text(input: &str) -> Result<String, &'static str> {
    let text = input.trim();
    if text.is_empty() || text.chars().count() > 4000 {
        return Err("消息需要 1 到 4000 个字符");
    }
    if text
        .chars()
        .any(|c| c != '\n' && c != '\t' && c.is_control())
    {
        return Err("消息包含不支持的字符");
    }
    Ok(text.to_string())
}

pub const RET_HOUR: i64 = 3600;
pub const RET_DAY: i64 = 86_400;
pub const RET_WEEK: i64 = 604_800;
pub const RET_MONTH: i64 = 2_592_000;
pub const GROUP_LIMIT: i64 = 100;
pub const SESSION_SECS: i64 = 14 * 86_400;
pub const MAX_UPLOAD: usize = 25 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 25_000_000;

pub fn retention_ok(secs: i64) -> bool {
    matches!(secs, RET_HOUR | RET_DAY | RET_WEEK | RET_MONTH)
}

pub fn preview(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        out
    }
}
