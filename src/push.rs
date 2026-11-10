use std::time::Duration;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hkdf::Hkdf;
use p256::ecdh::diffie_hellman;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use sha2::Sha256;

use crate::crypto::{self, Key};
use crate::error::AppErr;
use crate::util::{b64, now, random_bytes};

pub const PUSH_BODY: &str = "{\"title\":\"新消息\",\"body\":\"你有新消息\"}";

#[derive(Clone)]
pub struct Vapid {
    private: [u8; 32],
    pub public_b64: String,
}

impl Vapid {
    pub fn generate() -> Result<Self, AppErr> {
        let secret = SecretKey::random(&mut rand::rngs::OsRng);
        Self::from_bytes(secret.to_bytes().into())
    }

    pub fn seal(&self, key: &Key) -> Result<Vec<u8>, AppErr> {
        crypto::seal(key, b"vapid", &self.private)
    }

    pub fn open(key: &Key, packed: &[u8]) -> Result<Self, AppErr> {
        let bytes = crypto::open(key, b"vapid", packed)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| AppErr::Internal)?;
        Self::from_bytes(bytes)
    }

    fn from_bytes(private: [u8; 32]) -> Result<Self, AppErr> {
        let secret = SecretKey::from_bytes((&private).into()).map_err(|_| AppErr::Internal)?;
        let public = secret.public_key().to_encoded_point(false);
        let public = public.as_bytes();
        if public.len() != 65 || public[0] != 0x04 {
            return Err(AppErr::Internal);
        }
        Ok(Vapid {
            private,
            public_b64: b64(public),
        })
    }
}

pub struct PushSub {
    pub user_id: i64,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

pub async fn send(
    client: &reqwest::Client,
    vapid: &Vapid,
    subject: &str,
    sub: &PushSub,
) -> Result<(), PushGone> {
    let request = build_request(vapid, subject, sub)?;
    let response = client
        .post(request.url)
        .timeout(Duration::from_secs(10))
        .header("ttl", "86400")
        .header("urgency", "normal")
        .header("topic", "chat")
        .header("content-encoding", "aes128gcm")
        .header("content-type", "application/octet-stream")
        .header("authorization", request.authorization)
        .body(request.body)
        .send()
        .await
        .map_err(|err| {
            tracing::warn!(user_id = sub.user_id, error = %err, "推送失败");
            PushGone::Temporary
        })?;
    let status = response.status().as_u16();
    if status == 404 || status == 410 {
        return Err(PushGone::Remove);
    }
    if !(200..300).contains(&status) {
        tracing::warn!(user_id = sub.user_id, status, "推送被拒绝");
        return Err(PushGone::Temporary);
    }
    Ok(())
}

struct BuiltPush {
    url: String,
    authorization: String,
    body: Vec<u8>,
}

fn build_request(vapid: &Vapid, subject: &str, sub: &PushSub) -> Result<BuiltPush, PushGone> {
    let ua_public = URL_SAFE_NO_PAD
        .decode(sub.p256dh.trim())
        .map_err(|_| PushGone::Remove)?;
    let auth = URL_SAFE_NO_PAD
        .decode(sub.auth.trim())
        .map_err(|_| PushGone::Remove)?;
    if ua_public.len() != 65 || auth.len() != 16 {
        return Err(PushGone::Remove);
    }
    let body =
        encrypt_aes128gcm(&ua_public, &auth, PUSH_BODY.as_bytes()).map_err(|_| PushGone::Remove)?;
    let authorization =
        vapid_header(vapid, &sub.endpoint, subject).map_err(|_| PushGone::Temporary)?;
    Ok(BuiltPush {
        url: sub.endpoint.clone(),
        authorization,
        body,
    })
}

fn vapid_header(vapid: &Vapid, endpoint: &str, subject: &str) -> Result<String, ()> {
    let origin = endpoint_origin(endpoint)?;
    let header = b64(br#"{"typ":"JWT","alg":"ES256"}"#);
    let exp = now() + 12 * 3600;
    let payload_json = format!(r#"{{"aud":"{origin}","exp":{exp},"sub":"{subject}"}}"#);
    let payload = b64(payload_json.as_bytes());
    let input = format!("{header}.{payload}");
    let key = SigningKey::from_bytes((&vapid.private).into()).map_err(|_| ())?;
    let signature: Signature = key.sign(input.as_bytes());
    let token = format!("{input}.{}", b64(&signature.to_bytes()));
    Ok(format!("vapid t={token}, k={}", vapid.public_b64))
}

fn endpoint_origin(endpoint: &str) -> Result<String, ()> {
    let rest = endpoint.split_once("://").ok_or(())?;
    let host = rest.1.split('/').next().ok_or(())?;
    if host.is_empty() {
        return Err(());
    }
    Ok(format!("{}://{host}", rest.0))
}

fn encrypt_aes128gcm(ua_public: &[u8], auth: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, ()> {
    let ephemeral = SecretKey::random(&mut rand::rngs::OsRng);
    let as_public = ephemeral.public_key().to_encoded_point(false);
    let as_public = as_public.as_bytes();
    let peer = PublicKey::from_sec1_bytes(ua_public).map_err(|_| ())?;
    let shared = diffie_hellman(ephemeral.to_nonzero_scalar(), peer.as_affine());
    let ikm = derive_ikm(shared.raw_secret_bytes(), auth, ua_public, as_public);
    let salt = random_bytes::<16>();
    let (cek, nonce) = content_keys(&ikm, &salt)?;
    let mut record = plaintext.to_vec();
    record.push(0x02);
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|_| ())?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &record,
                aad: b"",
            },
        )
        .map_err(|_| ())?;
    let mut body = Vec::with_capacity(16 + 4 + 1 + as_public.len() + encrypted.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&4096u32.to_be_bytes());
    body.push(as_public.len() as u8);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&encrypted);
    Ok(body)
}

fn derive_ikm(shared: &[u8], auth: &[u8], ua_public: &[u8], as_public: &[u8]) -> [u8; 32] {
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(ua_public);
    info.extend_from_slice(as_public);
    let mut okm = [0u8; 32];
    let hk = Hkdf::<Sha256>::new(Some(auth), shared);
    hk.expand(&info, &mut okm)
        .expect("32 字节输出不会超过 HKDF 上限");
    okm
}

fn content_keys(ikm: &[u8], salt: &[u8]) -> Result<([u8; 16], [u8; 12]), ()> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|_| ())?;
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| ())?;
    Ok((cek, nonce))
}

#[derive(Debug)]
pub enum PushGone {
    Remove,
    Temporary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_encrypted_request() {
        let vapid = Vapid::generate().unwrap();
        let client = SecretKey::random(&mut rand::rngs::OsRng);
        let public = client.public_key().to_encoded_point(false);
        let sub = PushSub {
            user_id: 1,
            endpoint: "https://updates.push.services.mozilla.com/wpush/v2/example".into(),
            p256dh: b64(public.as_bytes()),
            auth: b64(&random_bytes::<16>()),
        };
        let built = build_request(&vapid, "https://chat.example", &sub).unwrap();
        assert!(built.authorization.starts_with("vapid t="));
        assert!(built.authorization.contains(&vapid.public_b64));
        assert_eq!(&built.body[16..20], &4096u32.to_be_bytes());
        assert_eq!(built.body[20], 65);
        assert_eq!(built.body[21], 0x04);
        assert!(built.body.len() > 86);
    }
}
