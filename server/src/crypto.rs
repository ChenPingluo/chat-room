use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::error::AppError;
use crate::util::fill_random;

#[derive(Clone)]
pub struct Crypto {
    key: [u8; 32],
}

impl Crypto {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub fn seal(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), AppError> {
        let cipher = self.cipher()?;
        let mut nonce_bytes = [0u8; 24];
        fill_random(&mut nonce_bytes)?;
        let nonce = XNonce::from(nonce_bytes);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| AppError::internal(anyhow::anyhow!("加密失败")))?;
        Ok((ciphertext, nonce_bytes.to_vec()))
    }

    pub fn open(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        if nonce.len() != 24 {
            return Err(AppError::internal(anyhow::anyhow!("nonce 长度错误")));
        }
        let cipher = self.cipher()?;
        let mut nonce_bytes = [0u8; 24];
        nonce_bytes.copy_from_slice(nonce);
        let nonce = XNonce::from(nonce_bytes);
        cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| AppError::internal(anyhow::anyhow!("解密失败")))
    }

    pub fn seal_file(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
        let (ciphertext, nonce) = self.seal(plaintext)?;
        let mut out = nonce;
        out.extend(ciphertext);
        Ok(out)
    }

    pub fn open_file(&self, blob: &[u8]) -> Result<Vec<u8>, AppError> {
        if blob.len() < 24 {
            return Err(AppError::internal(anyhow::anyhow!("图片文件已损坏")));
        }
        self.open(&blob[..24], &blob[24..])
    }

    fn cipher(&self) -> Result<XChaCha20Poly1305, AppError> {
        XChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|_| AppError::internal(anyhow::anyhow!("主密钥长度错误")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_file_blob() {
        let crypto = Crypto::new([9u8; 32]);
        let (ct, nonce) = crypto.seal(b"hello secret").unwrap();
        assert!(!ct.windows(5).any(|window| window == b"hello"));
        assert_eq!(crypto.open(&nonce, &ct).unwrap(), b"hello secret");
        let blob = crypto.seal_file(b"image-bytes").unwrap();
        assert_ne!(&blob[..4], b"imag");
        assert_eq!(crypto.open_file(&blob).unwrap(), b"image-bytes");
    }
}
