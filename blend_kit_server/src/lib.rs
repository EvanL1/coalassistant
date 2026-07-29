use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Json, State},
    http::{header, HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";
const SESSION_COOKIE: &str = "doudou_session";

pub fn app(public_dir: PathBuf) -> Router {
    app_with_auth(public_dir, AuthConfig::from_env())
}

#[derive(Clone)]
pub struct AuthConfig {
    username: String,
    password: String,
    session_token: String,
    secure_cookie: bool,
}

impl AuthConfig {
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
        session_token: impl Into<String>,
        secure_cookie: bool,
    ) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            session_token: session_token.into(),
            secure_cookie,
        }
    }

    pub fn from_env() -> Self {
        let on_railway = std::env::var_os("RAILWAY_ENVIRONMENT_NAME").is_some();
        let read = |name: &str, local_default: &str| {
            std::env::var(name).unwrap_or_else(|_| {
                if on_railway {
                    panic!("Railway 部署必须设置 {name}");
                }
                local_default.to_string()
            })
        };

        Self::new(
            read("AUTH_USERNAME", "doudou"),
            read("AUTH_PASSWORD", "123456"),
            read("AUTH_SESSION_TOKEN", "local-development-session"),
            on_railway,
        )
    }
}

pub fn app_with_auth(public_dir: PathBuf, auth: AuthConfig) -> Router {
    let index_file = public_dir.join("index.html");
    let static_files = ServeDir::new(public_dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index_file));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/session", get(auth_session))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/solve", post(solve))
        .route("/api/master", get(master))
        .route("/api/version", get(version))
        .fallback_service(static_files)
        .with_state(auth)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)],
        "ok",
    )
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct AuthStatus {
    authenticated: bool,
}

async fn auth_session(State(auth): State<AuthConfig>, headers: HeaderMap) -> Json<AuthStatus> {
    Json(AuthStatus {
        authenticated: is_authenticated(&headers, &auth),
    })
}

async fn login(State(auth): State<AuthConfig>, Json(input): Json<LoginRequest>) -> Response {
    if input.username != auth.username || !secret_eq(&input.password, &auth.password) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            r#"{"authenticated":false,"reason":"账号或密码错误"}"#,
        )
            .into_response();
    }

    let secure = if auth.secure_cookie { "; Secure" } else { "" };
    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400{secure}",
        auth.session_token
    );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, JSON_CONTENT_TYPE),
            (header::SET_COOKIE, cookie.as_str()),
        ],
        r#"{"authenticated":true}"#,
    )
        .into_response()
}

async fn logout() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, JSON_CONTENT_TYPE),
            (
                header::SET_COOKIE,
                "doudou_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
            ),
        ],
        r#"{"authenticated":false}"#,
    )
        .into_response()
}

async fn solve(
    State(auth): State<AuthConfig>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    if !is_authenticated(&headers, &auth) {
        return unauthorized();
    }

    let body = match axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024).await {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                format!(r#"{{"ok":false,"reason":"读取请求失败: {error}"}}"#),
            )
                .into_response();
        }
    };

    let input = match std::str::from_utf8(&body) {
        Ok(input) => input,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"ok":false,"reason":"请求体必须是 UTF-8 JSON"}"#,
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        blend_kit::solve_json(input),
    )
        .into_response()
}

async fn master(State(auth): State<AuthConfig>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &auth) {
        return unauthorized();
    }

    (
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        blend_kit::master_json(),
    )
        .into_response()
}

async fn version(State(auth): State<AuthConfig>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &auth) {
        return unauthorized();
    }

    (
        [(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)],
        env!("CARGO_PKG_VERSION"),
    )
        .into_response()
}

fn is_authenticated(headers: &HeaderMap, auth: &AuthConfig) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == SESSION_COOKIE).then_some(value)
            })
        })
        .is_some_and(|token| secret_eq(token, &auth.session_token))
}

fn secret_eq(actual: &str, expected: &str) -> bool {
    actual.len() == expected.len() && bool::from(actual.as_bytes().ct_eq(expected.as_bytes()))
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        r#"{"ok":false,"reason":"请先登录"}"#,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn test_app() -> Router {
        app_with_auth(
            PathBuf::from("missing-public"),
            AuthConfig::new("tester", "secret", "test-session-token", false),
        )
    }

    fn authenticated_request(method: &str, uri: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, "doudou_session=test-session-token")
            .body(body)
            .unwrap()
    }

    async fn response_text(response: Response) -> String {
        let body = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(body.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_text(response).await, "ok");
    }

    #[tokio::test]
    async fn test_solve_endpoint_preserves_json_boundary() {
        let response = test_app()
            .oneshot(authenticated_request(
                "POST",
                "/api/solve",
                Body::from("{not-json"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = serde_json::from_str(&response_text(response).await).unwrap();
        assert_eq!(result["ok"], false);
        assert!(result["reason"].as_str().unwrap().contains("JSON 解析失败"));
    }

    #[tokio::test]
    async fn test_master_and_version_endpoints() {
        let router = test_app();
        let master_response = router
            .clone()
            .oneshot(authenticated_request("GET", "/api/master", Body::empty()))
            .await
            .unwrap();
        let version_response = router
            .oneshot(authenticated_request("GET", "/api/version", Body::empty()))
            .await
            .unwrap();

        let master: Value = serde_json::from_str(&response_text(master_response).await).unwrap();
        assert!(master.is_object());
        assert_eq!(
            response_text(version_response).await,
            env!("CARGO_PKG_VERSION")
        );
    }

    #[tokio::test]
    async fn test_api_requires_login() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/api/master")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_login_sets_http_only_session_cookie() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"username":"tester","password":"secret"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains("doudou_session=test-session-token"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
    }

    #[tokio::test]
    async fn test_spa_routes_fall_back_to_index() {
        let public_dir = tempdir().unwrap();
        std::fs::write(
            public_dir.path().join("index.html"),
            "<html>豆哥配煤</html>",
        )
        .unwrap();

        let response = app(public_dir.path().to_path_buf())
            .oneshot(
                Request::builder()
                    .uri("/history")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_text(response).await, "<html>豆哥配煤</html>");
    }
}
