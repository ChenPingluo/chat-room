mod admin;
mod assets;
mod crypto;
mod db;
mod error;
mod files;
mod http;
mod hub;
mod images;
mod models;
mod push;
mod rate;
mod repo;
mod serve;
mod state;
mod totp;
mod util;

pub use serve::{serve, ServeConfig};
pub use state::{check, create_admin, App};

pub fn generate_passphrase() -> String {
    util::b64(&util::random_bytes::<32>())
}

pub fn totp_secret() -> Vec<u8> {
    totp::new_secret().to_vec()
}

pub fn totp_view(secret: &[u8], username: &str) -> Result<(String, String), &'static str> {
    totp::provisioning(secret, username).map_err(|err| err.message())
}

#[cfg(test)]
mod tests;
