#![forbid(unsafe_code)]

//! 邀请制私密聊天室。消息正文和图片在磁盘上加密，注册只能通过一次性邀请链接。

pub mod auth;
pub mod cli;
pub mod config;
pub mod crypto;
pub mod db;
pub mod domain;
pub mod dto;
pub mod error;
pub mod http;
pub mod hub;
pub mod janitor;
pub mod password;
pub mod state;
pub mod util;

pub use config::Config;
pub use error::AppError;
pub use state::AppState;

pub fn router(state: AppState) -> axum::Router {
    http::router(state)
}
