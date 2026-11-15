use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

pub fn serve(path: &str, fallback: &str, headers: &HeaderMap) -> Response {
    let requested = normalize(path);
    let (name, fallback_used) = match requested {
        Some(path)
            if Assets::get(&path).is_some() || Assets::get(&format!("{path}.gz")).is_some() =>
        {
            (path, false)
        }
        Some(path) if path.contains('.') => return StatusCode::NOT_FOUND.into_response(),
        _ => (fallback.to_string(), true),
    };
    let accepts_gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim().starts_with("gzip")));
    let gzip_name = format!("{name}.gz");
    let (bytes, gzip) = if accepts_gzip {
        if let Some(file) = Assets::get(&gzip_name) {
            (file.data, true)
        } else if let Some(file) = Assets::get(&name) {
            (file.data, false)
        } else {
            return StatusCode::NOT_FOUND.into_response();
        }
    } else if let Some(file) = Assets::get(&name) {
        (file.data, false)
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = bytes.into_response();
    let content_type = if name.ends_with(".webmanifest") {
        "application/manifest+json"
    } else if name.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else {
        mime_guess::from_path(&name)
            .first_raw()
            .unwrap_or("application/octet-stream")
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type_static(content_type)),
    );
    if gzip {
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    let cache = if !fallback_used && name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn content_type_static(value: &str) -> &'static str {
    match value {
        "application/manifest+json" => "application/manifest+json",
        "text/javascript; charset=utf-8" => "text/javascript; charset=utf-8",
        "text/css; charset=utf-8" => "text/css; charset=utf-8",
        "text/html" => "text/html",
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/svg+xml" => "image/svg+xml",
        "text/plain" => "text/plain",
        _ => "application/octet-stream",
    }
}

fn normalize(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    if path.contains("..") || path.contains('\\') || path.contains('\0') {
        return None;
    }
    Some(path.to_string())
}
