use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::domain::chat;
use crate::dto::{ConversationDto, MessageDto};
use crate::error::AppError;
use crate::http::JsonIn;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/conversations", get(list).post(create))
        .route("/conversations/{id}", get(get_one).patch(patch))
        .route(
            "/conversations/{id}/members",
            axum::routing::post(add_member),
        )
        .route(
            "/conversations/{id}/members/{user_id}",
            axum::routing::delete(kick),
        )
        .route("/conversations/{id}/leave", axum::routing::post(leave))
        .route(
            "/conversations/{id}/transfer",
            axum::routing::post(transfer),
        )
        .route("/conversations/{id}/read", axum::routing::post(mark_read))
        .route("/conversations/{id}/messages", get(messages).post(send))
        .route(
            "/conversations/{id}/messages/{message_id}",
            axum::routing::delete(delete_message),
        )
}

#[derive(Deserialize)]
struct CreateBody {
    kind: String,
    user_id: Option<i64>,
    name: Option<String>,
    member_ids: Option<Vec<i64>>,
}

#[derive(Deserialize)]
struct PatchBody {
    name: Option<String>,
    retention_secs: Option<i64>,
}

#[derive(Deserialize)]
struct UserBody {
    user_id: i64,
}

#[derive(Deserialize)]
struct ReadBody {
    message_id: i64,
}

#[derive(Deserialize)]
struct MessageQuery {
    before: Option<i64>,
    after: Option<i64>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct SendBody {
    kind: String,
    body: Option<String>,
    attachment_id: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<ConversationDto>>, AppError> {
    Ok(Json(chat::list_conversations(&state, user.id).await?))
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    JsonIn(body): JsonIn<CreateBody>,
) -> Result<(StatusCode, Json<ConversationDto>), AppError> {
    let conversation = match body.kind.as_str() {
        "direct" => {
            let other = body.user_id.ok_or_else(|| AppError::bad("缺少对方"))?;
            chat::create_direct(&state, &user, other).await?
        }
        "group" => {
            let name = body.name.as_deref().unwrap_or("");
            chat::create_group(
                &state,
                &user,
                name,
                body.member_ids.as_deref().unwrap_or(&[]),
            )
            .await?
        }
        _ => return Err(AppError::bad("会话类型只能是 direct 或 group")),
    };
    Ok((StatusCode::CREATED, Json(conversation)))
}

async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ConversationDto>, AppError> {
    Ok(Json(chat::get_conversation(&state, user.id, id).await?))
}

async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    JsonIn(body): JsonIn<PatchBody>,
) -> Result<Json<ConversationDto>, AppError> {
    Ok(Json(
        chat::patch_conversation(&state, &user, id, body.name.as_deref(), body.retention_secs)
            .await?,
    ))
}

async fn add_member(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    JsonIn(body): JsonIn<UserBody>,
) -> Result<Json<ConversationDto>, AppError> {
    Ok(Json(
        chat::add_member(&state, &user, id, body.user_id).await?,
    ))
}

async fn kick(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, user_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    chat::kick_member(&state, &user, id, user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    chat::leave(&state, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn transfer(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    JsonIn(body): JsonIn<UserBody>,
) -> Result<Json<ConversationDto>, AppError> {
    Ok(Json(chat::transfer(&state, &user, id, body.user_id).await?))
}

async fn mark_read(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    JsonIn(body): JsonIn<ReadBody>,
) -> Result<StatusCode, AppError> {
    chat::mark_read(&state, user.id, id, body.message_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn messages(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    if query.before.is_some() && query.after.is_some() {
        return Err(AppError::bad("before 和 after 不能同时使用"));
    }
    let messages = chat::list_messages(
        &state,
        user.id,
        id,
        query.before,
        query.after,
        query.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(serde_json::json!({ "messages": messages })))
}

async fn send(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    JsonIn(body): JsonIn<SendBody>,
) -> Result<(StatusCode, Json<MessageDto>), AppError> {
    let message = chat::send_message(
        &state,
        &user,
        id,
        &body.kind,
        body.body.as_deref(),
        body.attachment_id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(message)))
}

async fn delete_message(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, message_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    chat::delete_message(&state, &user, id, message_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
