use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::domain::social;
use crate::dto::{FriendRequestDto, FriendRequestLists, PublicUser};
use crate::error::AppError;
use crate::http::JsonIn;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users/search", get(search))
        .route("/friends", get(list_friends))
        .route("/friends/{user_id}", axum::routing::delete(remove_friend))
        .route("/friends/requests", get(list_requests).post(send_request))
        .route("/friends/requests/{id}/accept", axum::routing::post(accept))
        .route("/friends/requests/{id}/reject", axum::routing::post(reject))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Deserialize)]
struct UserIdBody {
    user_id: i64,
}

async fn search(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !search_allow(user.id) {
        return Err(AppError::too_many("搜索太频繁，请稍后再试"));
    }
    let users = social::search(&state, &user, &query.q).await?;
    Ok(Json(serde_json::json!({ "users": users })))
}

fn search_allow(user_id: i64) -> bool {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;
    use std::time::Duration;
    // 进程内滑动窗口。用户规模在一百人以内，这张表很小。
    static HITS: std::sync::OnceLock<Mutex<HashMap<i64, VecDeque<i64>>>> =
        std::sync::OnceLock::new();
    let map = HITS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("搜索限流锁");
    let now = crate::util::now_ms();
    let window = Duration::from_secs(60).as_millis() as i64;
    let queue = guard.entry(user_id).or_default();
    while queue.front().is_some_and(|stamp| now - *stamp > window) {
        queue.pop_front();
    }
    if queue.len() >= 20 {
        return false;
    }
    queue.push_back(now);
    true
}

async fn list_friends(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<PublicUser>>, AppError> {
    Ok(Json(social::list_friends(&state, &user).await?))
}

async fn send_request(
    State(state): State<AppState>,
    user: AuthUser,
    JsonIn(body): JsonIn<UserIdBody>,
) -> Result<(axum::http::StatusCode, Json<FriendRequestDto>), AppError> {
    let request = social::send_request(&state, &user, body.user_id).await?;
    Ok((axum::http::StatusCode::CREATED, Json(request)))
}

async fn list_requests(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<FriendRequestLists>, AppError> {
    Ok(Json(social::list_requests(&state, &user).await?))
}

async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<axum::http::StatusCode, AppError> {
    social::accept(&state, &user, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<axum::http::StatusCode, AppError> {
    social::reject(&state, &user, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn remove_friend(
    State(state): State<AppState>,
    user: AuthUser,
    Path(user_id): Path<i64>,
) -> Result<axum::http::StatusCode, AppError> {
    social::remove_friend(&state, &user, user_id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
