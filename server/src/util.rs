use std::io::Read;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::AppError;

pub const DEFAULT_RETENTION_SECS: i64 = 2_592_000;
pub const MAX_RETENTION_SECS: i64 = 2_592_000;
pub const SESSION_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
pub const INVITE_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const MAX_TEXT_CHARS: usize = 4000;
pub const MAX_GROUP_MEMBERS: i64 = 100;
pub const MAX_GROUP_NAME_CHARS: usize = 40;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn fill_random(buf: &mut [u8]) -> Result<(), AppError> {
    let mut file = std::fs::File::open("/dev/urandom").map_err(AppError::internal)?;
    file.read_exact(buf).map_err(AppError::internal)?;
    Ok(())
}

pub fn random_token() -> Result<String, AppError> {
    let mut buf = [0u8; 32];
    fill_random(&mut buf)?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

pub fn random_uid() -> Result<String, AppError> {
    let mut buf = [0u8; 4];
    fill_random(&mut buf)?;
    let value = u32::from_le_bytes(buf) % 9_000_000 + 1_000_000;
    Ok(value.to_string())
}

pub fn random_hex(bytes: usize) -> Result<String, AppError> {
    let mut buf = vec![0u8; bytes];
    fill_random(&mut buf)?;
    Ok(hex::encode(buf))
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn decode_master_key(text: &str) -> Result<[u8; 32], String> {
    let text = text.trim();
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .or_else(|_| STANDARD.decode(text))
        .map_err(|_| "主密钥不是有效的 base64".to_string())?;
    if bytes.len() != 32 {
        return Err("主密钥必须是 32 字节".to_string());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

pub fn valid_retention(secs: i64) -> bool {
    matches!(secs, 3600 | 86_400 | 604_800 | 2_592_000)
}

const RESERVED_USERNAMES: &[&str] = &[
    "admin",
    "administrator",
    "system",
    "support",
    "root",
    "owner",
    "help",
    "security",
    "official",
    "moderator",
    "mod",
    "null",
    "undefined",
];

pub fn validate_username(name: &str) -> Result<(), AppError> {
    let chars: Vec<char> = name.chars().collect();
    if !(3..=20).contains(&chars.len()) {
        return Err(AppError::bad("用户名需要 3 到 20 个字符"));
    }
    if !chars[0].is_ascii_alphabetic() {
        return Err(AppError::bad("用户名必须以字母开头"));
    }
    if !chars
        .iter()
        .all(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
    {
        return Err(AppError::bad("用户名只能包含字母、数字和下划线"));
    }
    if RESERVED_USERNAMES.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(AppError::bad("这个用户名不能使用"));
    }
    Ok(())
}

pub fn validate_group_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    let count = name.chars().count();
    if !(1..=MAX_GROUP_NAME_CHARS).contains(&count) {
        return Err(AppError::bad("群名需要 1 到 40 个字符"));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::bad("群名包含无效字符"));
    }
    Ok(name.to_string())
}

pub fn validate_text(text: &str) -> Result<(), AppError> {
    if text.chars().any(|ch| ch == '\0') {
        return Err(AppError::bad("消息包含无效字符"));
    }
    if text.trim().is_empty() {
        return Err(AppError::bad("消息不能为空"));
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(AppError::bad("消息不能超过 4000 个字符"));
    }
    Ok(())
}

pub fn session_cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-session"
    } else {
        "session"
    }
}

pub fn session_cookie(secure: bool, token: &str, max_age: i64) -> String {
    let name = session_cookie_name(secure);
    let mut value = format!("{name}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}");
    if secure {
        value.push_str("; Secure");
    }
    value
}

pub fn read_cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then_some(value)
    })
}

pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

/// 只在对端是本机（或测试里没有对端地址）时相信转发头。
/// 这样反代后面的真实 IP 能用于限流，直接暴露端口时伪造的头不会生效。
pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> String {
    let trust_forwarded = match peer {
        Some(ip) if ip.is_loopback() => true,
        None => true,
        Some(_) => false,
    };
    if trust_forwarded {
        if let Some(ip) = forwarded_ip(headers) {
            return ip;
        }
    }
    peer.map(|ip| ip.to_string())
        .unwrap_or_else(|| "local".into())
}

fn forwarded_ip(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
        })?;
    raw.parse::<IpAddr>().ok().map(|ip| ip.to_string())
}

pub fn origin_allowed(allowed: &str, headers: &HeaderMap, method_is_safe: bool) -> bool {
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if !method_is_safe && site == "cross-site" {
            return false;
        }
    }
    match headers.get(axum::http::header::ORIGIN) {
        None => true,
        Some(origin) => origin.to_str().ok() == Some(allowed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn username_rules() {
        assert!(validate_username("Ada").is_ok());
        assert!(validate_username("ab").is_err());
        assert!(validate_username("1abc").is_err());
        assert!(validate_username("root").is_err());
        assert!(validate_username("has space").is_err());
    }

    #[test]
    fn retention_is_discrete() {
        assert!(valid_retention(3600));
        assert!(valid_retention(2_592_000));
        assert!(!valid_retention(2_592_001));
        assert!(!valid_retention(10));
    }

    #[test]
    fn forwarded_ip_is_trusted_only_from_loopback() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.8, 10.0.0.1".parse().unwrap());
        assert_eq!(
            client_ip(&headers, Some(IpAddr::V4(Ipv4Addr::LOCALHOST))),
            "203.0.113.8"
        );
        assert_eq!(
            client_ip(&headers, Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)))),
            "198.51.100.2"
        );
        headers.insert("x-forwarded-for", "not-an-ip".parse().unwrap());
        assert_eq!(
            client_ip(&headers, Some(IpAddr::V4(Ipv4Addr::LOCALHOST))),
            "127.0.0.1"
        );
    }

    #[test]
    fn secure_cookie_uses_host_prefix() {
        let cookie = session_cookie(true, "abc", 10);
        assert!(cookie.starts_with("__Host-session=abc;"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(!session_cookie(false, "abc", 10).contains("Secure"));
    }
}
