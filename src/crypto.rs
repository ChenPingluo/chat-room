use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::error::AppErr;
use crate::util::random_bytes;

#[derive(Clone)]
pub struct Key(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

impl Key {
    pub fn derive(passphrase: &str, salt: &[u8]) -> Result<Self, AppErr> {
        let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|_| AppErr::Internal)?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut out = [0u8; 32];
        argon
            .hash_password_into(passphrase.as_bytes(), salt, &mut out)
            .map_err(|err| {
                tracing::error!(error = %err, "主密钥派生失败");
                AppErr::Internal
            })?;
        Ok(Key(Zeroizing::new(out)))
    }
}

pub fn hash_password(password: &str) -> Result<String, AppErr> {
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let params = Params::new(19 * 1024, 2, 1, None).map_err(|_| AppErr::Internal)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| {
            tracing::error!(error = %err, "密码哈希失败");
            AppErr::Internal
        })?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    let Ok(params) = Params::new(19 * 1024, 2, 1, None) else {
        return false;
    };
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon.verify_password(password.as_bytes(), &parsed).is_ok()
}

pub fn seal(key: &Key, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, AppErr> {
    let cipher =
        XChaCha20Poly1305::new_from_slice(key.0.as_slice()).map_err(|_| AppErr::Internal)?;
    let nonce = random_bytes::<24>();
    let encrypted = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
        .map_err(|_| AppErr::Internal)?;
    let mut out = Vec::with_capacity(nonce.len() + encrypted.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&encrypted);
    Ok(out)
}

pub fn open(key: &Key, aad: &[u8], packed: &[u8]) -> Result<Vec<u8>, AppErr> {
    if packed.len() < 24 + 16 {
        return Err(AppErr::Internal);
    }
    let (nonce, ciphertext) = packed.split_at(24);
    let cipher =
        XChaCha20Poly1305::new_from_slice(key.0.as_slice()).map_err(|_| AppErr::Internal)?;
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| AppErr::Internal)
}

pub fn open_startup(key: &Key, aad: &[u8], packed: &[u8]) -> bool {
    open(key, aad, packed).is_ok()
}
