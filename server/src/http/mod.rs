use std::net::SocketAddr;

use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequest, Request};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::{GovernorError, GovernorLayer};

use crate::error::AppError;
use crate::state::AppState;
use crate::util;

mod auth;
mod chat;
mod media;
mod social;
mod static_files;
mod ws;

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .merge(auth::routes())
        .merge(social::routes())
        .merge(chat::routes())
        .merge(media::routes())
        .layer(DefaultBodyLimit::max(64 * 1024));
    let uploads = Router::new()
        .route("/uploads", post(media::upload))
        .layer(DefaultBodyLimit::max(media::MAX_UPLOAD_BYTES + 64 * 1024));
    let secure = state.config.cookie_secure;
    let allowed = state.config.allowed_origin.clone();
    Router::new()
        .nest("/api", limit_api(api.merge(uploads)))
        .route("/ws", get(ws::connect))
        .fallback(static_files::fallback)
        .layer(middleware::from_fn(move |req, next| {
            let allowed = allowed.clone();
            async move { guard_origin(allowed, req, next).await }
        }))
        .layer(middleware::from_fn(move |req, next| async move {
            security_headers(secure, req, next).await
        }))
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(
            tower_http::trace::TraceLayer::new_for_http().make_span_with(|request: &Request| {
                tracing::info_span!(
                    "request",
                    method = %request.method(),
                    path = %sanitize_path(request.uri().path())
                )
            }),
        )
        .with_state(state)
}

fn sanitize_path(path: &str) -> String {
    if path.starts_with("/invite/") {
        "/invite/:token".to_string()
    } else {
        path.to_string()
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn guard_origin(allowed: String, request: Request, next: Next) -> Result<Response, AppError> {
    let method_is_safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !util::origin_allowed(&allowed, request.headers(), method_is_safe) {
        return Err(AppError::forbidden("跨站请求被拒绝"));
    }
    if request.uri().path() == "/ws" {
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            if origin.to_str().ok() != Some(allowed.as_str()) {
                return Err(AppError::forbidden("跨站请求被拒绝"));
            }
        }
    }
    Ok(next.run(request).await)
}

async fn security_headers(secure: bool, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    let mut csp = String::from(
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; font-src 'self'; manifest-src 'self'; worker-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'; object-src 'none'",
    );
    if secure {
        csp.push_str("; upgrade-insecure-requests");
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        );
    }
    if let Ok(value) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    if path.starts_with("/api") || path == "/ws" {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

pub struct JsonIn<T>(pub T);

impl<S, T> FromRequest<S> for JsonIn<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(JsonIn(value)),
            Err(_) => Err(AppError::bad("请求格式不正确")),
        }
    }
}

#[derive(Clone, Copy)]
struct ClientIpKeyExtractor;

impl KeyExtractor for ClientIpKeyExtractor {
    type Key = String;

    fn name(&self) -> &'static str {
        "client-ip"
    }

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip());
        Ok(util::client_ip(req.headers(), peer))
    }

    fn key_name(&self, key: &Self::Key) -> Option<String> {
        Some(key.clone())
    }
}

fn limit_api(router: Router<AppState>) -> Router<AppState> {
    let config = GovernorConfigBuilder::default()
        .per_millisecond(50)
        .burst_size(200)
        .key_extractor(ClientIpKeyExtractor)
        .finish()
        .expect("限流配置无效");
    router.layer(GovernorLayer::new(config).error_handler(|error| {
        match error {
            GovernorError::TooManyRequests { wait_time, headers } => {
                let mut response = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({
                        "error": format!("请求过于频繁，请 {wait_time} 秒后再试")
                    })),
                )
                    .into_response();
                if let Some(headers_in) = headers {
                    response.headers_mut().extend(headers_in);
                }
                response
            }
            _ => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({ "error": "请求过于频繁" })),
            )
                .into_response(),
        }
    }))
}
