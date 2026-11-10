use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{body::Body, Router};

use crate::auth::AuthUser;
use crate::domain::media;
pub use crate::domain::media::MAX_UPLOAD_BYTES;
use crate::error::AppError;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/attachments/{id}", get(download))
        .route("/attachments/{id}/thumb", get(download_thumb))
}

pub async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    mut multipart: Multipart,
) -> Result<(StatusCode, axum::Json<crate::dto::AttachmentDto>), AppError> {
    let mut bytes = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::bad("无法读取上传内容"))?
    {
        if field.name() == Some("file") {
            let data = field
                .bytes()
                .await
                .map_err(|_| AppError::bad("无法读取上传内容"))?;
            if data.len() > MAX_UPLOAD_BYTES {
                return Err(AppError::bad("图片不能超过 10MB"));
            }
            bytes = Some(data.to_vec());
            break;
        }
    }
    let bytes = bytes.ok_or_else(|| AppError::bad("缺少文件"))?;
    let attachment = media::store_upload(&state, user.id, bytes).await?;
    Ok((StatusCode::CREATED, axum::Json(attachment)))
}

async fn download(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, AppError> {
    send_image(&state, user.id, id, false).await
}

async fn download_thumb(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, AppError> {
    send_image(&state, user.id, id, true).await
}

async fn send_image(
    state: &AppState,
    user_id: i64,
    id: i64,
    thumb: bool,
) -> Result<axum::response::Response, AppError> {
    let bytes = media::read_for_user(state, user_id, id, thumb).await?;
    let mut response = Body::from(bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/webp"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline"),
    );
    Ok(response)
}
