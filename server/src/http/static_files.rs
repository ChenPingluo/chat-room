use axum::body::Body;
use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../web/dist/"]
struct Assets;

pub async fn fallback(uri: Uri, method: Method) -> Response {
    let path = uri.path();
    if path.starts_with("/api") || path == "/ws" {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "找不到"})),
        )
            .into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    match resolve(path) {
        Some(asset_path) => serve(&asset_path),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn resolve(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Some("index.html".to_string());
    }
    if Assets::get(path).is_some() {
        return Some(path.to_string());
    }
    if path.contains('.') {
        None
    } else {
        Some("index.html".to_string())
    }
}

fn serve(path: &str) -> Response {
    let Some(file) = Assets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = Body::from(file.data.into_owned()).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(content_type(path)) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(cache) {
        headers.insert(header::CACHE_CONTROL, value);
    }
    response
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("png") => "image/png",
        Some("txt") => "text/plain; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}
