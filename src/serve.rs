use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::watch;

use crate::error::StartupError;
use crate::http;
use crate::state::App;

pub struct ServeConfig {
    pub data_dir: std::path::PathBuf,
    pub passphrase: String,
    pub public_url: String,
    pub listen: SocketAddr,
    pub admin_listen: SocketAddr,
    pub insecure_cookies: bool,
}

pub async fn serve(config: ServeConfig) -> Result<(), StartupError> {
    if !config.admin_listen.ip().is_loopback() {
        return Err(StartupError::Message("管理端口只能监听本机地址"));
    }
    if !(config.public_url.starts_with("https://") || config.public_url.starts_with("http://")) {
        return Err(StartupError::Message("public-url 需要以 https:// 开头"));
    }
    if config.public_url.starts_with("http://") && !config.insecure_cookies {
        return Err(StartupError::Message(
            "http 地址只允许在显式开启 insecure-cookies 时用于本地测试",
        ));
    }
    let mut app = App::open(&config.data_dir, &config.passphrase, &config.public_url)?;
    app.insecure_cookies = config.insecure_cookies;
    let app = Arc::new(app);
    if !config.listen.ip().is_loopback() {
        tracing::warn!("聊天服务没有只监听本机，请确认前面有反向代理");
    }
    let public = http::wrap(http::router()).with_state(Arc::clone(&app));
    let admin = http::wrap(crate::admin::router()).with_state(Arc::clone(&app));
    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(StartupError::Io)?;
    let admin_listener = TcpListener::bind(config.admin_listen)
        .await
        .map_err(StartupError::Io)?;
    tracing::info!(listen = %config.listen, admin = %config.admin_listen, "服务已启动");
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let maintenance = {
        let app = Arc::clone(&app);
        let mut rx = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut sweep = tokio::time::interval(Duration::from_secs(60));
            let mut checkpoint = tokio::time::interval(Duration::from_secs(600));
            loop {
                tokio::select! {
                    changed = rx.changed() => {
                        if changed.is_err() || *rx.borrow() {
                            break;
                        }
                    }
                    _ = sweep.tick() => app.maintenance(),
                    _ = checkpoint.tick() => app.checkpoint(),
                }
            }
        })
    };
    let mut public_rx = shutdown_rx.clone();
    let mut admin_rx = shutdown_rx.clone();
    let public_server = tokio::spawn(async move {
        axum::serve(
            listener,
            public.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = public_rx.changed().await;
        })
        .await
    });
    let admin_server = tokio::spawn(async move {
        axum::serve(
            admin_listener,
            admin.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = admin_rx.changed().await;
        })
        .await
    });
    shutdown_signal().await;
    let _ = shutdown_tx.send(true);
    let _ = public_server.await;
    let _ = admin_server.await;
    maintenance.abort();
    app.checkpoint();
    tracing::info!("服务已停止");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
