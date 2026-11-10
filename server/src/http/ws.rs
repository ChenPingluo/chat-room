use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::auth::AuthUser;
use crate::domain::chat;
use crate::dto::ServerEvent;
use crate::error::AppError;
use crate::hub::Hub;
use crate::state::AppState;

pub async fn connect(
    State(state): State<AppState>,
    user: AuthUser,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    Ok(ws
        .max_frame_size(16 * 1024)
        .max_message_size(16 * 1024)
        .on_upgrade(move |socket| handle(socket, state, user)))
}

struct DropGuard {
    hub: Arc<Hub>,
    user_id: i64,
    conn_id: u64,
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.hub.disconnect(self.user_id, self.conn_id);
    }
}

async fn handle(socket: WebSocket, state: AppState, user: AuthUser) {
    let (conn_id, mut hub_rx) = match state.hub.connect(user.id) {
        Some(pair) => pair,
        None => {
            let (mut sink, _) = socket.split();
            let payload = serde_json::json!({"type":"error","error":"连接数已达上限"}).to_string();
            let _ = sink.send(Message::Text(payload.into())).await;
            return;
        }
    };
    let _guard = DropGuard {
        hub: state.hub.clone(),
        user_id: user.id,
        conn_id,
    };
    let (mut sink, mut stream) = socket.split();
    let (local_tx, mut local_rx) = mpsc::unbounded_channel::<String>();
    let ready = serde_json::to_string(&ServerEvent::Ready { user_id: user.id }).unwrap_or_default();
    let _ = local_tx.send(ready);
    let writer = tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                message = hub_rx.recv() => message,
                message = local_rx.recv() => message,
            };
            let Some(message) = next else { break };
            if sink.send(Message::Text(message.into())).await.is_err() {
                break;
            }
        }
    });
    let mut recent = std::collections::VecDeque::new();
    let mut violations = 0u32;
    while let Some(frame) = stream.next().await {
        let frame = match frame {
            Ok(frame) => frame,
            Err(_) => break,
        };
        let text = match frame {
            Message::Text(text) => text.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        if text.len() > 16 * 1024 {
            let _ = local_tx.send(error_event(None, "消息太大"));
            continue;
        }
        let now = crate::util::now_ms();
        while recent.front().is_some_and(|stamp| now - *stamp > 1000) {
            recent.pop_front();
        }
        if recent.len() >= 10 {
            violations += 1;
            let _ = local_tx.send(error_event(None, "发送太快了"));
            if violations >= 5 {
                break;
            }
            continue;
        }
        recent.push_back(now);
        violations = 0;
        handle_text(&state, &user, &local_tx, &text).await;
    }
    writer.abort();
}

async fn handle_text(
    state: &AppState,
    user: &AuthUser,
    local_tx: &mpsc::UnboundedSender<String>,
    text: &str,
) {
    let frame: ClientFrame = match serde_json::from_str(text) {
        Ok(frame) => frame,
        Err(_) => {
            let _ = local_tx.send(error_event(None, "无法识别的消息"));
            return;
        }
    };
    match frame.frame_type.as_str() {
        "ping" => {
            let _ = local_tx.send(serde_json::json!({"type":"pong"}).to_string());
        }
        "send" => {
            let conv_id = frame.conv_id.unwrap_or(0);
            let kind = frame.kind.unwrap_or_else(|| "text".into());
            match chat::send_message(
                state,
                user,
                conv_id,
                &kind,
                frame.body.as_deref(),
                frame.attachment_id,
            )
            .await
            {
                Ok(mut message) => {
                    message.client_msg_id = frame.client_msg_id;
                    let _ = local_tx.send(
                        serde_json::to_string(&ServerEvent::Message { message })
                            .unwrap_or_default(),
                    );
                }
                Err(err) => {
                    let _ = local_tx.send(error_event(frame.client_msg_id, &err.to_string()));
                }
            }
        }
        _ => {
            let _ = local_tx.send(error_event(frame.client_msg_id, "未知的消息类型"));
        }
    }
}

fn error_event(client_msg_id: Option<String>, error: &str) -> String {
    serde_json::to_string(&ServerEvent::Error {
        client_msg_id,
        error: error.to_string(),
    })
    .unwrap_or_else(|_| "{\"type\":\"error\",\"error\":\"发送失败\"}".to_string())
}

#[derive(Deserialize)]
struct ClientFrame {
    #[serde(rename = "type")]
    frame_type: String,
    conv_id: Option<i64>,
    client_msg_id: Option<String>,
    kind: Option<String>,
    body: Option<String>,
    attachment_id: Option<i64>,
}
