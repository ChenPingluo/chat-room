use totp_rs::{Algorithm, TOTP};

use crate::error::AppErr;
use crate::util::random_bytes;

pub fn new_secret() -> [u8; 20] {
    random_bytes()
}

fn make(secret: &[u8], username: &str) -> Result<TOTP, AppErr> {
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret.to_vec(),
        Some("chat-room".into()),
        username.into(),
    )
    .map_err(|err| {
        tracing::error!(error = %err, "两步验证初始化失败");
        AppErr::Internal
    })
}

pub fn provisioning(secret: &[u8], username: &str) -> Result<(String, String), AppErr> {
    let totp = make(secret, username)?;
    Ok((totp.get_secret_base32(), totp.get_url()))
}

pub fn verify(secret: &[u8], username: &str, code: &str) -> Result<bool, AppErr> {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Ok(false);
    }
    make(secret, username)?
        .check_current(&code)
        .map_err(|_| AppErr::Internal)
}
