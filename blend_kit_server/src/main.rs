use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode};

use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
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
    let auth = match blend_kit_server::AuthConfig::from_env() {
        Ok(auth) => auth,
        Err(reason) => {
            error!("{reason}, 服务没有启动");
            return ExitCode::FAILURE;
        }
    };
    if auth.uses_default_credentials() {
        warn!(
            "没有显式设置 AUTH_USERNAME / AUTH_PASSWORD / AUTH_SESSION_TOKEN, 用的是内置默认口令: \
             只监听 127.0.0.1 (本机开发够用). 三项都设置后才对外监听"
        );
    }
    let address = SocketAddr::new(auth.listen_ip(), port);
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            error!(%address, %error, "无法监听, 服务没有启动");
            return ExitCode::FAILURE;
        }
    };

    info!(%address, static_dir = %public_dir.display(), "豆哥配煤服务已启动");

    // 带上 TCP 对端地址: 没有代理头时 (本机直连) 登录限速按它区分来源.
    let app = match blend_kit_server::app(public_dir, auth).await {
        Ok(app) => app,
        Err(reason) => {
            error!("{reason}, 服务没有启动");
            return ExitCode::FAILURE;
        }
    };
    let served = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(%error, "HTTP 服务异常退出");
            ExitCode::FAILURE
        }
    }
}

/// 等停止信号 (Ctrl+C 或 SIGTERM). 哪个信号装不上监听就记一条错误、当它永远不来:
/// 服务照常运行, 只是那个信号不能触发优雅退出.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = signal::ctrl_c().await {
            error!(%error, "无法监听 Ctrl+C");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(error) => {
                error!(%error, "无法监听 SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("收到停止信号，正在优雅退出");
}
