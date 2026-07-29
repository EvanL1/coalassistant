use std::{env, net::SocketAddr, path::PathBuf};

use tokio::signal;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3000);
    let public_dir = env::var_os("STATIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("public"));
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .unwrap_or_else(|error| panic!("无法监听 {address}: {error}"));

    info!(%address, static_dir = %public_dir.display(), "豆哥配煤服务已启动");

    axum::serve(listener, blend_kit_server::app(public_dir).await)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("HTTP 服务异常退出");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("无法监听 Ctrl+C");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("无法监听 SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("收到停止信号，正在优雅退出");
}
