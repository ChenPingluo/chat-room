use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::AppError;

const COMMON: &[&str] = &[
    "password",
    "password1",
    "password12",
    "password123",
    "password1234",
    "123456789",
    "1234567890",
    "12345678901",
    "qwertyuiop",
    "qwerty123",
    "qwerty1234",
    "iloveyou",
    "iloveyou1",
    "admin12345",
    "letmein123",
    "welcome123",
    "changeme123",
    "abcdefghij",
    "1111111111",
    "0000000000",
    "passw0rd",
    "passw0rd12",
    "qwertyuiop1",
    "1q2w3e4r5t",
    "1qaz2wsx3e",
];

pub fn validate_password(password: &str, username: &str) -> Result<(), AppError> {
    let chars = password.chars().count();
    if chars < 10 {
        return Err(AppError::bad("密码至少 10 个字符"));
    }
    if chars > 128 || password.len() > 512 {
        return Err(AppError::bad("密码过长"));
    }
    if password.eq_ignore_ascii_case(username) {
        return Err(AppError::bad("密码不能和用户名相同"));
    }
    let lower = password.to_lowercase();
    if COMMON.iter().any(|item| *item == lower) {
        return Err(AppError::bad("密码太常见"));
    }
    Ok(())
}

fn hasher() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).expect("argon2 参数");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| AppError::internal(anyhow::anyhow!("密码哈希失败: {err}")))
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    hasher()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_and_common_passwords() {
        assert!(validate_password("short", "ada").is_err());
        assert!(validate_password("password123", "ada").is_err());
        assert!(validate_password("AdaUserName", "adausername").is_err());
        assert!(validate_password("correct horse", "ada").is_ok());
    }

    #[test]
    fn hash_verifies_only_the_same_password() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("correct horse!", &hash));
    }
}
