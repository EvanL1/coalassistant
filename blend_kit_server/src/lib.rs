use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Json, Path, State},
    http::{header, HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

mod coal_index;
mod database;
mod master_data;

const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";
const SESSION_COOKIE: &str = "doudou_session";
/// 数据更新接口的机器密钥请求头. 与用户会话 Cookie 完全独立 ——
/// 这把钥匙只能写煤库覆盖层, 碰不到用户数据与历史。
const DATA_API_KEY_HEADER: &str = "x-api-key";
/// 数据更新接口的请求体上限. 合法批次最多两百余条 (见 master_data::MAX_UPDATES),
/// 256KB 绰绰有余。
const MAX_DATA_API_BODY: usize = 256 * 1024;
const MAX_UPDATED_BY_CHARS: usize = 64;

pub async fn app(public_dir: PathBuf) -> Router {
    let database = database::connect_from_env().await;
    app_with_state(
        public_dir,
        AppState {
            auth: AuthConfig::from_env(),
            database,
            data_api_key: data_api_key_from_env(),
        },
    )
}

/// DATA_API_KEY 未设置或过短则视为未配置, 数据更新接口保持关闭.
/// 32 字符下限与 AUTH_SESSION_TOKEN 的要求一致, 避免弱密钥被暴力枚举。
fn data_api_key_from_env() -> Option<String> {
    let key = std::env::var("DATA_API_KEY").ok()?;
    let key = key.trim().to_string();
    // 按字符数而非字节数, 与文档口径一致 (否则 11 个中文字符 = 33 字节即可通过).
    if key.chars().count() < 32 {
        if !key.is_empty() {
            tracing::warn!("DATA_API_KEY 短于 32 字符, 数据更新接口未启用");
        }
        return None;
    }
    Some(key)
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

#[derive(Clone)]
struct AppState {
    auth: AuthConfig,
    database: Option<PgPool>,
    /// 数据更新接口的机器密钥. 未设置 DATA_API_KEY 时为 None, 接口整体关闭 ——
    /// 默认不开放写入口, 要用必须显式配置。
    data_api_key: Option<String>,
}

pub fn app_with_auth(public_dir: PathBuf, auth: AuthConfig) -> Router {
    app_with_state(
        public_dir,
        AppState {
            auth,
            database: None,
            data_api_key: None,
        },
    )
}

fn app_with_state(public_dir: PathBuf, state: AppState) -> Router {
    let index_file = public_dir.join("index.html");
    let static_files = ServeDir::new(public_dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index_file));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/coal-index", get(coal_index_handler))
        .route("/api/auth/session", get(auth_session))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/solve", post(solve))
        .route("/api/master", get(master))
        // 数据更新接口: 机器密钥 (X-API-Key), 与用户会话无关
        .route("/api/master/coals", post(update_coal_data))
        .route("/api/master/overrides", get(list_coal_overrides))
        .route(
            "/api/master/coals/{coal}/{field}",
            delete(delete_coal_override),
        )
        .route("/api/version", get(version))
        .route("/api/storage", get(get_storage).put(put_storage))
        .route(
            "/api/history",
            get(list_history).post(create_history).delete(clear_history),
        )
        .route("/api/history/count", get(count_history))
        .route("/api/history/import", post(import_history))
        .route("/api/history/{id}/measured", patch(set_measured_quality))
        .fallback_service(static_files)
        .with_state(state)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

async fn health(State(state): State<AppState>) -> Response {
    if let Some(pool) = &state.database {
        if let Err(error) = database::health(pool).await {
            tracing::error!(%error, "PostgreSQL 健康检查失败");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)],
                "database unavailable",
            )
                .into_response();
        }
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)],
        "ok",
    )
        .into_response()
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

/// 焦煤现货指数序列, 供今日屏参考估算. 公开(不敏感, 与 health 同级).
async fn coal_index_handler() -> Response {
    match coal_index::get_spot_series().await {
        Some(points) => Json(points).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)],
            "coal index unavailable",
        )
            .into_response(),
    }
}

async fn auth_session(State(state): State<AppState>, headers: HeaderMap) -> Json<AuthStatus> {
    Json(AuthStatus {
        authenticated: is_authenticated(&headers, &state.auth),
    })
}

async fn login(State(state): State<AppState>, Json(input): Json<LoginRequest>) -> Response {
    let auth = &state.auth;
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
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    if !is_authenticated(&headers, &state.auth) {
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

async fn master(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }

    // 基线 (编译期嵌入, 进 git) + 覆盖层 (PG, 由数据更新接口写入).
    // 没连库或读失败时退回纯基线 —— 煤库能少几个补充指标, 但不能整个不可用.
    if let Some(pool) = state.database.as_ref() {
        match database::list_coal_overrides(pool).await {
            Ok(overrides) if !overrides.is_empty() => match master_data::merge(&overrides) {
                Ok(merged) => {
                    return ([(header::CONTENT_TYPE, JSON_CONTENT_TYPE)], merged).into_response()
                }
                Err(error) => tracing::error!(%error, "煤库覆盖层合并失败, 退回基线"),
            },
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "读取煤库覆盖层失败, 退回基线"),
        }
    }

    (
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        blend_kit::master_json(),
    )
        .into_response()
}

async fn version(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }

    (
        [(header::CONTENT_TYPE, TEXT_CONTENT_TYPE)],
        env!("CARGO_PKG_VERSION"),
    )
        .into_response()
}

async fn get_storage(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::load_storage(pool, &username).await {
        Ok(storage) => Json(storage).into_response(),
        Err(error) => database_error(error),
    }
}

async fn put_storage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut storage): Json<database::UserStorage>,
) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    if let Err(reason) = storage.validate() {
        return bad_request(reason);
    }
    storage.initialized = true;
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::save_storage(pool, &username, &storage).await {
        Ok(()) => Json(storage).into_response(),
        Err(error) => database_error(error),
    }
}

async fn create_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<database::CreateHistory>,
) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    if let Err(reason) = input.validate() {
        return bad_request(reason);
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::create_history(pool, &username, &input).await {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(error) => database_error(error),
    }
}

async fn import_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<database::ImportHistory>,
) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    if input.entries.len() > 100 {
        return bad_request("一次最多导入 100 条历史方案");
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::import_history(pool, &username, &input).await {
        Ok(imported) => Json(json!({ "imported": imported })).into_response(),
        Err(error) => database_error(error),
    }
}

async fn count_history(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::count_history(pool, &username).await {
        Ok(count) => Json(database::count_response(count)).into_response(),
        Err(error) => database_error(error),
    }
}

async fn list_history(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::list_history(pool, &username).await {
        Ok(history) => Json(history).into_response(),
        Err(error) => database_error(error),
    }
}

async fn set_measured_quality(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(measured): Json<database::MeasuredQuality>,
) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    if let Err(reason) = measured.validate() {
        return bad_request(reason);
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::set_measured_quality(pool, &username, &id, &measured).await {
        Ok(true) => Json(json!({ "updated": true })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            r#"{"ok":false,"reason":"历史方案不存在"}"#,
        )
            .into_response(),
        Err(error) => database_error(error),
    }
}

async fn clear_history(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::clear_history(pool, &username).await {
        Ok(deleted) => Json(json!({ "deleted": deleted })).into_response(),
        Err(error) => database_error(error),
    }
}

fn authenticated_username(headers: &HeaderMap, state: &AppState) -> Result<String, Box<Response>> {
    is_authenticated(headers, &state.auth)
        .then(|| state.auth.username.clone())
        .ok_or_else(|| Box::new(unauthorized()))
}

fn require_database(state: &AppState) -> Result<&PgPool, Box<Response>> {
    state.database.as_ref().ok_or_else(|| {
        Box::new(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"ok":false,"reason":"数据库尚未配置"}"#,
            )
                .into_response(),
        )
    })
}

fn bad_request(reason: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        Json(json!({ "ok": false, "reason": reason })),
    )
        .into_response()
}

fn database_error(error: sqlx::Error) -> Response {
    tracing::error!(%error, "PostgreSQL 操作失败");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        r#"{"ok":false,"reason":"数据库操作失败"}"#,
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

// ---------------------------------------------------------------------------
// 煤库数据更新接口 (机器密钥, 与用户会话完全独立)
// ---------------------------------------------------------------------------

/// 校验 X-API-Key. 未配置 DATA_API_KEY 时接口整体关闭 (503), 不是放行。
fn check_data_api_key(headers: &HeaderMap, state: &AppState) -> Result<(), Box<Response>> {
    let Some(expected) = state.data_api_key.as_ref() else {
        return Err(Box::new(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"ok":false,"reason":"数据更新接口未启用 (未配置 DATA_API_KEY)"}"#,
            )
                .into_response(),
        ));
    };
    let presented = headers
        .get(DATA_API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if secret_eq(presented, expected) {
        Ok(())
    } else {
        // 只记事件, 不记 presented —— 那是攻击者可控内容, 不该进日志.
        tracing::warn!("数据更新接口密钥校验失败");
        Err(Box::new(
            (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"ok":false,"reason":"X-API-Key 无效"}"#,
            )
                .into_response(),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct CoalUpdateRequest {
    updates: Vec<master_data::CoalUpdate>,
    /// 可选的调用方标识, 只用于落库留痕 (谁写的这批数据).
    #[serde(default)]
    updated_by: Option<String>,
}

/// POST /api/master/coals —— 批量补充煤库指标.
///
/// 只填空: 基线 coal_master.json 已有的指标一律拒绝, 整批校验不通过则一条不写。
///
/// 这里收的是 `Request<Body>` 而不是 `Json<T>` extractor: axum 的 extractor 在
/// handler 函数体**之前**执行, 用 `Json<T>` 会让未鉴权的请求先被反序列化 ——
/// 既把接口 schema 回显给任何访客 (`updates[0].coal: invalid type ...`),
/// 又让鉴权前就能触发 2MB 反序列化。同文件 `solve` 出于同样理由这么写。
async fn update_coal_data(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    if let Err(response) = check_data_api_key(&headers, &state) {
        return *response;
    }

    let body = match axum::body::to_bytes(request.into_body(), MAX_DATA_API_BODY).await {
        Ok(body) => body,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "reason": "请求体读取失败或超出大小上限" })),
            )
                .into_response();
        }
    };
    let payload: CoalUpdateRequest = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "reason": format!("请求体不是合法 JSON: {error}") })),
            )
                .into_response();
        }
    };

    // 先校验再要数据库: payload 有问题时调用方应当拿到 422 而不是 503,
    // 否则会把"自己数据写错了"误判成"服务端挂了"。
    if let Err(errors) = master_data::validate(&payload.updates) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "ok": false, "errors": errors })),
        )
            .into_response();
    }

    let updated_by = payload.updated_by.as_deref().unwrap_or("api");
    if updated_by.chars().count() > MAX_UPDATED_BY_CHARS {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "errors": [format!("updated_by 不得超过 {MAX_UPDATED_BY_CHARS} 字符")]
            })),
        )
            .into_response();
    }

    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };

    match database::upsert_coal_overrides(pool, &payload.updates, updated_by).await {
        Ok(applied) => Json(json!({ "ok": true, "applied": applied })).into_response(),
        Err(error) => database_error(error),
    }
}

/// GET /api/master/overrides —— 查看当前覆盖层内容 (排查"这个值哪来的")。
async fn list_coal_overrides(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = check_data_api_key(&headers, &state) {
        return *response;
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::list_coal_overrides(pool).await {
        Ok(rows) => Json(json!({ "ok": true, "overrides": rows })).into_response(),
        Err(error) => database_error(error),
    }
}

/// DELETE /api/master/coals/{coal}/{field} —— 回滚一个值到基线状态。
async fn delete_coal_override(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((coal, field)): Path<(String, String)>,
) -> Response {
    if let Err(response) = check_data_api_key(&headers, &state) {
        return *response;
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::delete_coal_override(pool, &coal, &field).await {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "reason": "覆盖层中没有这一条" })),
        )
            .into_response(),
        Ok(removed) => Json(json!({ "ok": true, "removed": removed })).into_response(),
        Err(error) => database_error(error),
    }
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

    /// 配好机器密钥的 app (无数据库 —— 鉴权应在碰库之前就判完).
    fn test_app_with_key(key: &str) -> Router {
        app_with_state(
            PathBuf::from("missing-public"),
            AppState {
                auth: AuthConfig::new("tester", "secret", "test-session-token", false),
                database: None,
                data_api_key: Some(key.to_string()),
            },
        )
    }

    fn key_request(method: &str, uri: &str, key: Option<&str>, body: Body) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(key) = key {
            builder = builder.header(DATA_API_KEY_HEADER, key);
        }
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap()
    }

    const TEST_KEY: &str = "0123456789abcdef0123456789abcdef";

    /// 未配置 DATA_API_KEY 时接口整体关闭, 而不是放行.
    #[tokio::test]
    async fn test_data_api_disabled_without_key() {
        let response = test_app()
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                Some("任意值"),
                Body::from(r#"{"updates":[]}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// 缺密钥、错密钥都必须 401, 且不能因为登录了用户会话就放行.
    #[tokio::test]
    async fn test_data_api_rejects_missing_or_wrong_key() {
        let router = test_app_with_key(TEST_KEY);
        for key in [None, Some("wrong-key-wrong-key-wrong-key-123")] {
            let response = router
                .clone()
                .oneshot(key_request(
                    "POST",
                    "/api/master/coals",
                    key,
                    Body::from(r#"{"updates":[]}"#),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "key={key:?}");
        }

        // 用户会话 Cookie 不是这把钥匙的替代品
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/master/coals")
                    .header(header::COOKIE, "doudou_session=test-session-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"updates":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 密钥正确但数据不合法 -> 422, 且错误信息要指出问题.
    #[tokio::test]
    async fn test_data_api_validates_before_touching_database() {
        let body = serde_json::to_string(&json!({
            "updates": [
                { "coal": "不存在的煤", "field": "S", "value": 1.0, "source": "测试" }
            ]
        }))
        .unwrap();
        let response = test_app_with_key(TEST_KEY)
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                Some(TEST_KEY),
                Body::from(body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let text = response_text(response).await;
        assert!(text.contains("没有这个煤名"), "实际: {text}");
    }

    /// 岩相=2026 这类脏数据在入库前就被拦下 (2026-09-17 真实案例).
    #[tokio::test]
    async fn test_data_api_rejects_out_of_range_value() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| !c.props.contains_key("CSR") && !c.props.is_empty())
            .unwrap();
        let body = serde_json::to_string(&json!({
            "updates": [
                { "coal": target.name, "field": "CSR", "value": 2026.0, "source": "脏数据" }
            ]
        }))
        .unwrap();
        let response = test_app_with_key(TEST_KEY)
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                Some(TEST_KEY),
                Body::from(body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response_text(response).await.contains("超出物理量程"));
    }

    /// 畸形请求体在**鉴权之前**不得被解析 —— 否则 axum 的 Json extractor 会把
    /// 接口 schema (字段名/类型) 回显给任何未鉴权访客, 且能在鉴权前触发反序列化。
    /// 这三种请求都必须先撞上密钥检查, 而不是拿到 400/415/422 的解析错误。
    #[tokio::test]
    async fn test_malformed_body_does_not_bypass_auth() {
        let router = test_app_with_key(TEST_KEY);

        // 坏 JSON + 无密钥 -> 401 (而不是 400 "Failed to parse the request body")
        let response = router
            .clone()
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                None,
                Body::from("{not json"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let text = response_text(response).await;
        assert!(!text.contains("updates"), "不得回显字段名: {text}");

        // 类型错误 + 无密钥 -> 401 (而不是 422 "updates[0].coal: invalid type")
        let response = router
            .clone()
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                None,
                Body::from(r#"{"updates":[{"coal":1}]}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!response_text(response).await.contains("invalid type"));

        // 无 Content-Type + 无密钥 -> 401 (而不是 415)
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/master/coals")
                    .body(Body::from(r#"{"updates":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 未配置密钥时, 畸形请求体同样只能拿到 503, 不能泄露任何 schema.
    #[tokio::test]
    async fn test_disabled_api_leaks_nothing() {
        let response = test_app()
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                None,
                Body::from(r#"{"updates":[{"coal":1}]}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!response_text(response).await.contains("invalid type"));
    }

    /// 单批条数上限: 挡住"一次几万条"的错误放大.
    #[tokio::test]
    async fn test_rejects_oversized_batch() {
        let updates: Vec<_> = (0..master_data::MAX_UPDATES + 1)
            .map(
                |i| json!({ "coal": format!("煤{i}"), "field": "CSR", "value": 60, "source": "x" }),
            )
            .collect();
        let body = serde_json::to_string(&json!({ "updates": updates })).unwrap();

        let response = test_app_with_key(TEST_KEY)
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                Some(TEST_KEY),
                Body::from(body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let text = response_text(response).await;
        assert!(text.contains("一次最多提交"), "实际: {text}");
        // 超限时只回一条错误, 不是逐条生成
        assert!(!text.contains("煤299"), "不应逐条报错: {text}");
    }

    /// source 过长会被写进 note 并随每次 GET /api/master 回给前端, 必须封顶.
    #[tokio::test]
    async fn test_rejects_overlong_source() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| !c.props.is_empty() && !c.props.contains_key("CSR"))
            .unwrap();
        let body = serde_json::to_string(&json!({
            "updates": [{
                "coal": target.name, "field": "CSR", "value": 60,
                "source": "来".repeat(500)
            }]
        }))
        .unwrap();

        let response = test_app_with_key(TEST_KEY)
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                Some(TEST_KEY),
                Body::from(body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response_text(response).await.contains("source 不得超过"));
    }

    /// 查询与回滚接口同样受密钥保护.
    #[tokio::test]
    async fn test_override_read_and_delete_need_key() {
        let router = test_app_with_key(TEST_KEY);
        for (method, uri) in [
            ("GET", "/api/master/overrides"),
            ("DELETE", "/api/master/coals/临北/CSR"),
        ] {
            let response = router
                .clone()
                .oneshot(key_request(method, uri, None, Body::empty()))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }
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
    async fn test_storage_requires_database_after_login() {
        let response = test_app()
            .oneshot(authenticated_request("GET", "/api/storage", Body::empty()))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
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

        let response = app_with_auth(
            public_dir.path().to_path_buf(),
            AuthConfig::new("tester", "secret", "test-session-token", false),
        )
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
