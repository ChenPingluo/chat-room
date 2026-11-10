use std::io::Cursor;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, ImageReader, Limits};
use sqlx::FromRow;

use crate::dto::AttachmentDto;
use crate::error::AppError;
use crate::state::AppState;
use crate::util::{self, now_ms};

pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
const MAX_EDGE: u32 = 5_000;
const STORED_EDGE: u32 = 2_048;
const THUMB_EDGE: u32 = 320;

pub struct Processed {
    pub full: Vec<u8>,
    pub thumb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn process_image(bytes: &[u8]) -> Result<Processed, AppError> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::bad("图片不能超过 10MB"));
    }
    let format = sniff(bytes).ok_or_else(|| AppError::bad("只接受 JPEG、PNG、WebP 或 GIF"))?;
    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.set_format(format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| AppError::bad("无法识别的图片，或图片尺寸过大"))?;
    let pixels = u64::from(decoded.width()) * u64::from(decoded.height());
    if pixels == 0 || pixels > MAX_PIXELS {
        return Err(AppError::bad("图片尺寸过大"));
    }
    let full_image = fit(decoded, STORED_EDGE);
    let width = full_image.width();
    let height = full_image.height();
    let thumb = fit(full_image.clone(), THUMB_EDGE);
    Ok(Processed {
        full: encode_webp(&full_image)?,
        thumb: encode_webp(&thumb)?,
        width,
        height,
    })
}

fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else {
        None
    }
}

fn fit(image: DynamicImage, max_edge: u32) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    if width.max(height) <= max_edge {
        return image;
    }
    let (target_w, target_h) = if width >= height {
        let scaled = u64::from(height) * u64::from(max_edge) / u64::from(width);
        (max_edge, scaled.max(1) as u32)
    } else {
        let scaled = u64::from(width) * u64::from(max_edge) / u64::from(height);
        (scaled.max(1) as u32, max_edge)
    };
    image.resize_exact(target_w, target_h, image::imageops::FilterType::Triangle)
}

fn encode_webp(image: &DynamicImage) -> Result<Vec<u8>, AppError> {
    let rgba = image.to_rgba8();
    let mut buffer = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(rgba)
        .write_to(&mut buffer, ImageFormat::WebP)
        .map_err(|err| AppError::internal(anyhow::anyhow!("图片编码失败: {err}")))?;
    Ok(buffer.into_inner())
}

#[derive(FromRow)]
#[allow(dead_code)]
struct AttachmentRow {
    id: i64,
    uploader_id: i64,
    file_name: String,
    thumb_file_name: String,
    size: i64,
    width: i64,
    height: i64,
    message_id: Option<i64>,
}

pub async fn store_upload(
    state: &AppState,
    user_id: i64,
    bytes: Vec<u8>,
) -> Result<AttachmentDto, AppError> {
    let _permit = state
        .images
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::internal(anyhow::anyhow!("图片队列已关闭")))?;
    let processed = tokio::task::spawn_blocking(move || process_image(&bytes))
        .await
        .map_err(AppError::internal)??;
    let file_name = format!("{}.bin", util::random_hex(16)?);
    let thumb_file_name = format!("{}.bin", util::random_hex(16)?);
    let full = state.crypto.seal_file(&processed.full)?;
    let thumb = state.crypto.seal_file(&processed.thumb)?;
    write_private(&state.files_dir().join(&file_name), &full)?;
    if let Err(err) = write_private(&state.files_dir().join(&thumb_file_name), &thumb) {
        let _ = std::fs::remove_file(state.files_dir().join(&file_name));
        return Err(err);
    }
    let now = now_ms();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO attachments (uploader_id, file_name, thumb_file_name, mime, size, width, height, created_at)
         VALUES (?, ?, ?, 'image/webp', ?, ?, ?, ?) RETURNING id",
    )
    .bind(user_id)
    .bind(&file_name)
    .bind(&thumb_file_name)
    .bind(i64::try_from(processed.full.len()).unwrap_or(i64::MAX))
    .bind(i64::from(processed.width))
    .bind(i64::from(processed.height))
    .bind(now)
    .fetch_one(&state.db)
    .await?;
    Ok(AttachmentDto {
        id,
        width: i64::from(processed.width),
        height: i64::from(processed.height),
        size: i64::try_from(processed.full.len()).unwrap_or(0),
    })
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::internal)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(AppError::internal)?;
        std::io::Write::write_all(&mut file, bytes).map_err(AppError::internal)?;
        file.sync_all().map_err(AppError::internal)?;
    }
    std::fs::rename(&tmp, path).map_err(AppError::internal)?;
    Ok(())
}

pub fn remove_files(dir: &Path, file_name: &str, thumb_file_name: &str) {
    let _ = std::fs::remove_file(dir.join(file_name));
    let _ = std::fs::remove_file(dir.join(thumb_file_name));
}

pub async fn read_for_user(
    state: &AppState,
    user_id: i64,
    attachment_id: i64,
    thumb: bool,
) -> Result<Vec<u8>, AppError> {
    let row = sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, uploader_id, file_name, thumb_file_name, size, width, height, message_id FROM attachments WHERE id = ?",
    )
    .bind(attachment_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    if row.message_id.is_none() {
        if row.uploader_id != user_id {
            return Err(AppError::NotFound);
        }
    } else {
        let conv_id: Option<i64> = sqlx::query_scalar(
            "SELECT conv_id FROM messages WHERE attachment_id = ? AND deleted_at IS NULL",
        )
        .bind(attachment_id)
        .fetch_optional(&state.db)
        .await?;
        let Some(conv_id) = conv_id else {
            return Err(AppError::NotFound);
        };
        let member: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM conversation_members WHERE conv_id = ? AND user_id = ?",
        )
        .bind(conv_id)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?;
        if member.is_none() {
            return Err(AppError::NotFound);
        }
    }
    let name = if thumb {
        &row.thumb_file_name
    } else {
        &row.file_name
    };
    let path: PathBuf = state.files_dir().join(name);
    let encrypted = tokio::fs::read(&path)
        .await
        .map_err(|_| AppError::NotFound)?;
    state.crypto.open_file(&encrypted)
}

pub async fn delete_attachment(state: &AppState, attachment_id: i64) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, AttachmentRow>(
        "SELECT id, uploader_id, file_name, thumb_file_name, size, width, height, message_id FROM attachments WHERE id = ?",
    )
    .bind(attachment_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    sqlx::query("UPDATE messages SET attachment_id = NULL WHERE attachment_id = ?")
        .bind(attachment_id)
        .execute(&state.db)
        .await?;
    sqlx::query("DELETE FROM attachments WHERE id = ?")
        .bind(attachment_id)
        .execute(&state.db)
        .await?;
    remove_files(&state.files_dir(), &row.file_name, &row.thumb_file_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reencodes_png_to_webp_without_original_bytes() {
        let image = image::RgbaImage::from_pixel(12, 8, image::Rgba([10, 20, 30, 255]));
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let original = png.into_inner();
        let processed = process_image(&original).unwrap();
        assert!(processed.full.starts_with(b"RIFF"));
        assert_eq!(&processed.full[8..12], b"WEBP");
        assert_ne!(processed.full, original);
        assert!(processed.width <= STORED_EDGE);
        assert!(process_image(b"<svg>not an image</svg>").is_err());
    }
}
