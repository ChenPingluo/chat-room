pub mod auth;
pub mod chat;
pub mod media;
pub mod social;

use crate::dto::ServerEvent;
use crate::state::AppState;

pub(crate) fn emit(state: &AppState, user_ids: &[i64], event: &ServerEvent) {
    match serde_json::to_string(event) {
        Ok(payload) => state.hub.emit(user_ids, payload),
        Err(err) => tracing::error!(%err, "事件序列化失败"),
    }
}
