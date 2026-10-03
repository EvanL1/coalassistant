use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

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

mod admin;
mod api_key;
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
/// 管理员会话令牌最短字节数. 令牌即 Cookie 值, 太短可被穷举.
const MIN_ADMIN_TOKEN_BYTES: usize = 32;
/// 登录失败后的固定等待, 拖慢在线暴力猜测 (不做按 IP 计数).
const LOGIN_FAILURE_DELAY: Duration = Duration::from_secs(1);

pub async fn app(public_dir: PathBuf) -> Router {
    let database = database::connect_from_env().await;
    // k 启动时读一次, 之后只由 PUT /api/admin/settings 更新 —— 求解路径不碰库.
    let rank_k = match database.as_ref() {
        Some(pool) => admin::load_rank_interaction_k(pool).await,
        None => None,
    };
    app_with_state(
        public_dir,
        AppState {
            auth: AuthConfig::from_env(),
            database,
            rank_k: Arc::new(RwLock::new(rank_k)),
        },
    )
}

/// 密钥名字上限, 与迁移里的 CHECK 一致.
const MAX_KEY_NAME_CHARS: usize = 64;

#[derive(Clone)]
pub struct AuthConfig {
    username: String,
    password: String,
    session_token: String,
    secure_cookie: bool,
    /// 管理员账号. None = 管理端停用 (ADMIN_* 未配齐).
    admin: Option<AdminAccount>,
}

/// 管理员账号: 普通账号的超集, 另可调参、维护煤库、管理密钥.
#[derive(Clone)]
struct AdminAccount {
    username: String,
    password: String,
    session_token: String,
}

/// 会话角色. 管理员会话对普通接口同样有效.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    User,
    Admin,
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
            admin: None,
        }
    }

    /// 启用管理员账号. 以下情况拒绝启用 (记错误日志, 管理端停用):
    /// - 令牌与普通会话令牌相同: 无法区分角色, 普通用户的 Cookie 会直接拿到管理权限;
    /// - 账号密码与普通账号完全相同: 普通凭据登录即成管理员;
    /// - 令牌短于 [`MIN_ADMIN_TOKEN_BYTES`]: 可被穷举.
    pub fn with_admin(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
        session_token: impl Into<String>,
    ) -> Self {
        let admin = AdminAccount {
            username: username.into(),
            password: password.into(),
            session_token: session_token.into(),
        };
        let refusal = if admin.session_token == self.session_token {
            Some("ADMIN_SESSION_TOKEN 与 AUTH_SESSION_TOKEN 相同")
        } else if admin.username == self.username && admin.password == self.password {
            Some("管理员账号密码与普通账号相同")
        } else if admin.session_token.len() < MIN_ADMIN_TOKEN_BYTES {
            Some("ADMIN_SESSION_TOKEN 短于 32 字节")
        } else {
            None
        };
        match refusal {
            Some(reason) => {
                tracing::error!("{reason}, 管理端停用");
                self.admin = None;
            }
            None => self.admin = Some(admin),
        }
        self
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

        let config = Self::new(
            read("AUTH_USERNAME", "doudou"),
            read("AUTH_PASSWORD", "123456"),
            read("AUTH_SESSION_TOKEN", "local-development-session"),
            on_railway,
        );
        // 管理员三项全部可选: 缺任一项即停用管理端, Railway 上也不 panic.
        let optional = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        match (
            optional("ADMIN_USERNAME"),
            optional("ADMIN_PASSWORD"),
            optional("ADMIN_SESSION_TOKEN"),
        ) {
            (Some(username), Some(password), Some(token)) => {
                config.with_admin(username, password, token)
            }
            _ => {
                tracing::info!("ADMIN_* 未配齐, 管理端停用");
                config
            }
        }
    }
}

#[derive(Clone)]
struct AppState {
    auth: AuthConfig,
    database: Option<PgPool>,
    /// 管理端设置的煤阶交互 k (内存缓存). 单实例部署; 多实例时其他实例要重启才生效.
    rank_k: Arc<RwLock<Option<f64>>>,
}

impl AppState {
    fn rank_k(&self) -> Option<f64> {
        *self
            .rank_k
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_rank_k(&self, k: Option<f64>) {
        *self
            .rank_k
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = k;
    }
}

pub fn app_with_auth(public_dir: PathBuf, auth: AuthConfig) -> Router {
    app_with_state(
        public_dir,
        AppState {
            auth,
            database: None,
            rank_k: Arc::default(),
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
        // 数据更新接口: 机器密钥 (X-API-Key) 或管理员会话, 普通用户会话无权
        .route("/api/master/coals", post(update_coal_data))
        .route("/api/master/overrides", get(list_coal_overrides))
        .route(
            "/api/master/coals/{coal}/{field}",
            delete(delete_coal_override),
        )
        // 管理端: 只认管理员会话 (不是 X-API-Key)
        .route("/api/admin/keys", get(list_api_keys).post(create_api_key))
        .route("/api/admin/keys/{id}", delete(revoke_api_key))
        .route(
            "/api/admin/settings",
            get(admin::get_settings).put(admin::put_settings),
        )
        .route("/api/admin/calibration", get(admin::calibration))
        .route("/api/admin/csr-model", get(admin::csr_model))
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
    admin: bool,
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
    let role = session_role(&headers, &state.auth);
    Json(AuthStatus {
        authenticated: role.is_some(),
        admin: role == Some(Role::Admin),
    })
}

async fn login(State(state): State<AppState>, Json(input): Json<LoginRequest>) -> Response {
    let auth = &state.auth;
    // 账号也按常数时间比较, 且两组凭据都完整比完再判 (`&` 不短路),
    // 免得响应时间泄露哪个账号存在.
    let matches = |username: &str, password: &str| {
        secret_eq(&input.username, username) & secret_eq(&input.password, password)
    };
    let admin_ok = auth
        .admin
        .as_ref()
        .map(|admin| matches(&admin.username, &admin.password));
    let user_ok = matches(&auth.username, &auth.password);
    let (token, body) = match (&auth.admin, admin_ok) {
        (Some(admin), Some(true)) => (
            &admin.session_token,
            r#"{"authenticated":true,"admin":true}"#,
        ),
        _ if user_ok => (
            &auth.session_token,
            r#"{"authenticated":true,"admin":false}"#,
        ),
        _ => {
            tokio::time::sleep(LOGIN_FAILURE_DELAY).await;
            return (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"authenticated":false,"reason":"账号或密码错误"}"#,
            )
                .into_response();
        }
    };

    let secure = if auth.secure_cookie { "; Secure" } else { "" };
    let cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400{secure}"
    );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, JSON_CONTENT_TYPE),
            (header::SET_COOKIE, cookie.as_str()),
        ],
        body,
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

    // 管理端设了煤阶交互 k 时, 对没自带 rank_interaction 的请求注入; 没设时原样求解.
    // k 取内存缓存, 不碰库.
    let input = admin::inject_rank_interaction(input, state.rank_k());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        blend_kit::solve_json(&input),
    )
        .into_response()
}

async fn master(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }

    (
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        merged_master_json(&state).await,
    )
        .into_response()
}

/// 基线 (编译期嵌入, 进 git) + 覆盖层 (PG, 由数据更新接口写入).
/// 没连库或读失败时退回纯基线 —— 煤库能少几个补充指标, 但不能整个不可用.
async fn merged_master_json(state: &AppState) -> std::borrow::Cow<'static, str> {
    if let Some(pool) = state.database.as_ref() {
        match database::list_coal_overrides(pool).await {
            Ok(overrides) if !overrides.is_empty() => match master_data::merge(&overrides) {
                Ok(merged) => return merged.into(),
                Err(error) => tracing::error!(%error, "煤库覆盖层合并失败, 退回基线"),
            },
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "读取煤库覆盖层失败, 退回基线"),
        }
    }
    blend_kit::master_json().into()
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
    session_role(headers, auth).is_some()
}

fn session_role(headers: &HeaderMap, auth: &AuthConfig) -> Option<Role> {
    let token = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == SESSION_COOKIE).then_some(value)
            })
        })?;
    if auth
        .admin
        .as_ref()
        .is_some_and(|admin| secret_eq(token, &admin.session_token))
    {
        return Some(Role::Admin);
    }
    secret_eq(token, &auth.session_token).then_some(Role::User)
}

/// 管理端守卫: 未登录 401, 普通账号 403. 成功时返回管理员账号 (留痕用).
fn require_admin<'a>(
    headers: &HeaderMap,
    state: &'a AppState,
) -> Result<&'a AdminAccount, Box<Response>> {
    match (session_role(headers, &state.auth), &state.auth.admin) {
        (Some(Role::Admin), Some(admin)) => Ok(admin),
        (None, _) => Err(Box::new(unauthorized())),
        _ => Err(Box::new(
            (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"ok":false,"reason":"需要管理员权限"}"#,
            )
                .into_response(),
        )),
    }
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

/// 数据更新接口的鉴权: 管理员会话或有效 X-API-Key 二选一.
/// 返回落库留痕用的持有者名字 (管理员为 `管理员:<账号>`, 密钥为密钥名字),
/// 以及是否走的是 Cookie 会话 (写接口据此要求 JSON Content-Type).
async fn check_data_writer(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<(String, bool), Box<Response>> {
    if let (Some(Role::Admin), Some(admin)) =
        (session_role(headers, &state.auth), &state.auth.admin)
    {
        return Ok((format!("管理员:{}", admin.username), true));
    }
    check_data_api_key(headers, state)
        .await
        .map(|holder| (holder, false))
}

/// Cookie 鉴权的写接口必须带 `Content-Type: application/json`, 否则 415.
/// 浏览器跨站"简单请求"发不出这个头, 多一道 CSRF 防线 (SameSite=Strict 之外).
fn require_json_content_type(headers: &HeaderMap) -> Result<(), Box<Response>> {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"));
    if is_json {
        return Ok(());
    }
    Err(Box::new(
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            r#"{"ok":false,"reason":"Content-Type 须为 application/json"}"#,
        )
            .into_response(),
    ))
}

/// 校验 X-API-Key: 对来访明文做 SHA-256, 再查未销毁的密钥行.
/// 成功时返回该密钥的名字, 供落库留痕使用 (调用方伪造不了).
async fn check_data_api_key(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<String, Box<Response>> {
    let unauthorized = || {
        // 只记事件, 不记 presented —— 那是攻击者可控内容, 不该进日志.
        tracing::warn!("数据更新接口密钥校验失败");
        Box::new(
            (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"ok":false,"reason":"X-API-Key 无效或已销毁"}"#,
            )
                .into_response(),
        )
    };

    let presented = headers
        .get(DATA_API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if presented.is_empty() {
        return Err(unauthorized());
    }

    let pool = require_database(state)?;
    let hash = api_key::hash_key(presented);
    match database::authenticate_api_key(pool, &hash).await {
        Ok(Some(name)) => Ok(name),
        Ok(None) => Err(unauthorized()),
        Err(error) => {
            tracing::error!(%error, "密钥校验查询失败");
            Err(Box::new(database_error(error)))
        }
    }
}

// ---------------------------------------------------------------------------
// 管理端: 密钥的生成 / 列出 / 销毁
//
// 这三个接口走**管理员会话**(登录 Cookie), 不是 X-API-Key —— 管钥匙的不能是钥匙
// 自己, 否则一把泄露的密钥就能给自己发新的。
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateKeyRequest {
    name: String,
}

/// POST /api/admin/keys —— 生成一把新密钥. 明文只在这一次响应里出现.
async fn create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateKeyRequest>,
) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    let name = payload.name.trim();
    if name.is_empty() || name.chars().count() > MAX_KEY_NAME_CHARS {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "reason": format!("名字不能为空, 且不超过 {MAX_KEY_NAME_CHARS} 字符")
            })),
        )
            .into_response();
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };

    let generated = api_key::generate();
    match database::create_api_key(pool, name, &generated.hash, &generated.prefix).await {
        Ok(row) => {
            tracing::info!(name, "已生成数据更新密钥");
            (
                StatusCode::CREATED,
                Json(json!({
                    "ok": true,
                    "key": row,
                    // 唯一一次返回明文: 库里只有哈希, 之后取不回来
                    "plaintext": generated.plaintext,
                })),
            )
                .into_response()
        }
        Err(error) => database_error(error),
    }
}

/// GET /api/admin/keys —— 列出密钥 (只有名字/前缀/时间, 永不含明文或哈希).
async fn list_api_keys(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::list_api_keys(pool).await {
        Ok(keys) => Json(json!({ "ok": true, "keys": keys })).into_response(),
        Err(error) => database_error(error),
    }
}

/// DELETE /api/admin/keys/{id} —— 销毁一把密钥 (软删除, 立即失效).
async fn revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::revoke_api_key(pool, &id).await {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "reason": "密钥不存在或已销毁" })),
        )
            .into_response(),
        Ok(_) => {
            tracing::info!(%id, "已销毁数据更新密钥");
            Json(json!({ "ok": true })).into_response()
        }
        Err(error) => database_error(error),
    }
}

#[derive(Debug, Deserialize)]
struct CoalUpdateRequest {
    updates: Vec<master_data::CoalUpdate>,
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
    // 持有者名字由密钥推导, 不接受调用方自报 —— 出了坏数据要查得到人.
    let holder = match check_data_writer(&headers, &state).await {
        Ok((holder, by_session)) => {
            if by_session {
                if let Err(response) = require_json_content_type(&headers) {
                    return *response;
                }
            }
            holder
        }
        Err(response) => return *response,
    };

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

    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };

    match database::upsert_coal_overrides(pool, &payload.updates, &holder).await {
        Ok(applied) => {
            tracing::info!(%holder, applied, "煤库覆盖层已更新");
            Json(json!({ "ok": true, "applied": applied, "updated_by": holder })).into_response()
        }
        Err(error) => database_error(error),
    }
}

/// GET /api/master/overrides —— 查看当前覆盖层内容 (排查"这个值哪来的")。
async fn list_coal_overrides(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = check_data_writer(&headers, &state).await {
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
    if let Err(response) = check_data_writer(&headers, &state).await {
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
            AuthConfig::new("tester", "secret", "test-session-token", false).with_admin(
                "boss",
                "admin-secret",
                "admin-session-token-0123456789abcdef",
            ),
        )
    }

    /// 未配置管理员的应用 (对应 ADMIN_* 缺失).
    fn app_without_admin() -> Router {
        app_with_auth(
            PathBuf::from("missing-public"),
            AuthConfig::new("tester", "secret", "test-session-token", false),
        )
    }

    fn session_request(method: &str, uri: &str, token: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, format!("doudou_session={token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap()
    }

    async fn login_as(router: Router, username: &str, password: &str) -> Response {
        router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({ "username": username, "password": password }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// 普通账号与管理员账号登录, 拿到各自的令牌与角色.
    #[tokio::test]
    async fn test_login_roles() {
        let response = login_as(test_app(), "tester", "secret").await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.contains("doudou_session=test-session-token"));
        let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
        assert_eq!(body, json!({ "authenticated": true, "admin": false }));

        let response = login_as(test_app(), "boss", "admin-secret").await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.contains("doudou_session=admin-session-token-0123456789abcdef"));
        assert!(cookie.contains("HttpOnly"));
        let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
        assert_eq!(body, json!({ "authenticated": true, "admin": true }));

        // 管理员账号配普通密码不行
        let response = login_as(test_app(), "boss", "secret").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_session_reports_role() {
        for (token, expected) in [
            (
                "test-session-token",
                json!({ "authenticated": true, "admin": false }),
            ),
            (
                "admin-session-token-0123456789abcdef",
                json!({ "authenticated": true, "admin": true }),
            ),
            ("forged", json!({ "authenticated": false, "admin": false })),
        ] {
            let response = test_app()
                .oneshot(session_request(
                    "GET",
                    "/api/auth/session",
                    token,
                    Body::empty(),
                ))
                .await
                .unwrap();
            let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
            assert_eq!(body, expected, "token={token}");
        }
    }

    /// 管理员会话是超集: 普通接口照样能用.
    #[tokio::test]
    async fn test_admin_session_works_on_user_endpoints() {
        let response = test_app()
            .oneshot(session_request(
                "GET",
                "/api/version",
                "admin-session-token-0123456789abcdef",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// 管理端守卫: 未登录 401, 普通账号 403, 管理员放行 (没库时 503).
    #[tokio::test]
    async fn test_admin_guard_status_codes() {
        let routes: [(&str, &str, &str); 6] = [
            ("GET", "/api/admin/settings", ""),
            ("PUT", "/api/admin/settings", r#"{"rank_interaction_k":50}"#),
            ("GET", "/api/admin/calibration", ""),
            ("GET", "/api/admin/csr-model", ""),
            ("GET", "/api/admin/keys", ""),
            ("DELETE", "/api/admin/keys/some-id", ""),
        ];
        for (method, uri, body) in routes {
            let anonymous = test_app()
                .oneshot(key_request(method, uri, None, Body::from(body)))
                .await
                .unwrap();
            assert_eq!(
                anonymous.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );

            let user = test_app()
                .oneshot(session_request(
                    method,
                    uri,
                    "test-session-token",
                    Body::from(body),
                ))
                .await
                .unwrap();
            assert_eq!(user.status(), StatusCode::FORBIDDEN, "{method} {uri}");
            let text = response_text(user).await;
            assert!(text.contains("需要管理员权限"), "{text}");

            let admin = test_app()
                .oneshot(session_request(
                    method,
                    uri,
                    "admin-session-token-0123456789abcdef",
                    Body::from(body),
                ))
                .await
                .unwrap();
            assert_eq!(
                admin.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{method} {uri}"
            );
        }
    }

    /// 设置校验在碰库之前: 非法 k 一律 400.
    #[tokio::test]
    async fn test_put_settings_rejects_invalid_k() {
        for body in [
            r#"{"rank_interaction_k":-1}"#,
            r#"{"rank_interaction_k":"x"}"#,
            "{}",
        ] {
            let response = test_app()
                .oneshot(session_request(
                    "PUT",
                    "/api/admin/settings",
                    "admin-session-token-0123456789abcdef",
                    Body::from(body),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "body={body}");
        }
    }

    /// 管理员令牌与普通令牌相同时不启用管理端, 普通 Cookie 拿不到管理权限.
    #[tokio::test]
    async fn test_admin_token_collision_disables_admin() {
        let router = app_with_auth(
            PathBuf::from("missing-public"),
            AuthConfig::new(
                "tester",
                "secret",
                "same-session-token-0123456789abcdef",
                false,
            )
            .with_admin(
                "boss",
                "admin-secret",
                "same-session-token-0123456789abcdef",
            ),
        );
        let response = router
            .oneshot(session_request(
                "GET",
                "/api/admin/settings",
                "same-session-token-0123456789abcdef",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// 账号密码与普通账号完全相同、或令牌太短时不启用管理端.
    #[tokio::test]
    async fn test_with_admin_refuses_weak_config() {
        let long_token = "admin-session-token-0123456789abcdef";
        for auth in [
            AuthConfig::new("tester", "secret", "test-session-token", false)
                .with_admin("tester", "secret", long_token),
            AuthConfig::new("tester", "secret", "test-session-token", false).with_admin(
                "boss",
                "admin-secret",
                "short-admin-token",
            ),
        ] {
            assert!(auth.admin.is_none());
        }

        // 同账号同密码: 普通凭据登录只能得到普通会话
        let router = app_with_auth(
            PathBuf::from("missing-public"),
            AuthConfig::new("tester", "secret", "test-session-token", false)
                .with_admin("tester", "secret", long_token),
        );
        let response = login_as(router, "tester", "secret").await;
        let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
        assert_eq!(body, json!({ "authenticated": true, "admin": false }));

        // 同账号不同密码是允许的
        let auth = AuthConfig::new("tester", "secret", "test-session-token", false).with_admin(
            "tester",
            "admin-secret",
            long_token,
        );
        assert!(auth.admin.is_some());
    }

    /// 登录失败固定等待约 1 秒, 拖慢暴力猜测.
    #[tokio::test]
    async fn test_failed_login_is_delayed() {
        let started = std::time::Instant::now();
        let response = login_as(test_app(), "tester", "wrong").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(started.elapsed() >= LOGIN_FAILURE_DELAY - Duration::from_millis(50));
    }

    const SOLVE_FIXTURE: &str = r#"{
        "coals": [
            {"name": "焦煤", "props": {"S": 0.5, "A": 9, "V": 24, "G": 85, "Y": 16, "petro": 0.1, "CSR": 66, "M": 8}, "fob": 1200, "frt": 0},
            {"name": "瘦煤", "props": {"S": 0.5, "A": 9, "V": 14, "G": 40, "Y": 8, "petro": 0.1, "CSR": 55, "M": 8}, "fob": 700, "frt": 0}
        ],
        "specs": [{"indicator": "CSR", "direction": "Lower", "min": 60, "enforcement": "Hard", "enabled": true}],
        "truncate_decimal": false
    }"#;

    async fn solve_with_cached_k(k: Option<f64>) -> Value {
        let router = app_with_state(
            PathBuf::from("missing-public"),
            AppState {
                auth: AuthConfig::new("tester", "secret", "test-session-token", false),
                database: None,
                rank_k: Arc::new(RwLock::new(k)),
            },
        );
        let response = router
            .oneshot(authenticated_request(
                "POST",
                "/api/solve",
                Body::from(SOLVE_FIXTURE),
            ))
            .await
            .unwrap();
        serde_json::from_str(&response_text(response).await).unwrap()
    }

    /// 求解从内存缓存取 k (此处没有数据库): 有 k 时注入, 没有时不计交互.
    #[tokio::test]
    async fn test_solve_uses_cached_rank_k() {
        let with_k = solve_with_cached_k(Some(66.0)).await;
        assert_eq!(with_k["ok"], true);
        assert!(with_k["csr_interaction_penalty"].as_f64().is_some());

        let without = solve_with_cached_k(None).await;
        assert_eq!(without["ok"], true);
        assert!(without["csr_interaction_penalty"].is_null());
    }

    /// Cookie 鉴权的写接口缺 JSON Content-Type 一律 415 (鉴权之后、碰库之前).
    #[tokio::test]
    async fn test_admin_cookie_writes_require_json_content_type() {
        for (method, uri, body) in [
            ("POST", "/api/master/coals", r#"{"updates":[]}"#),
            ("PUT", "/api/admin/settings", r#"{"rank_interaction_k":50}"#),
        ] {
            for content_type in [
                None,
                Some("text/plain"),
                Some("application/x-www-form-urlencoded"),
            ] {
                let mut builder = Request::builder().method(method).uri(uri).header(
                    header::COOKIE,
                    "doudou_session=admin-session-token-0123456789abcdef",
                );
                if let Some(content_type) = content_type {
                    builder = builder.header(header::CONTENT_TYPE, content_type);
                }
                let response = test_app()
                    .oneshot(builder.body(Body::from(body)).unwrap())
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "{method} {uri} {content_type:?}"
                );
            }
        }

        // 带参数的 JSON 媒体类型照常放行
        let response = test_app()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/admin/settings")
                    .header(
                        header::COOKIE,
                        "doudou_session=admin-session-token-0123456789abcdef",
                    )
                    .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
                    .body(Body::from(r#"{"rank_interaction_k":-1}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// 没配管理员: 管理员凭据登录失败, 普通账号访问管理端 403.
    #[tokio::test]
    async fn test_admin_disabled_without_config() {
        let response = login_as(app_without_admin(), "boss", "admin-secret").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app_without_admin()
            .oneshot(session_request(
                "GET",
                "/api/admin/settings",
                "test-session-token",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = app_without_admin()
            .oneshot(session_request(
                "GET",
                "/api/admin/settings",
                "admin-session-token-0123456789abcdef",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 管理员会话可代替 X-API-Key 写煤库: 鉴权通过后才轮到请求体校验 (422).
    #[tokio::test]
    async fn test_admin_session_passes_data_api_auth() {
        let response = test_app()
            .oneshot(session_request(
                "POST",
                "/api/master/coals",
                "admin-session-token-0123456789abcdef",
                Body::from(r#"{"updates":[]}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        for (method, uri) in [
            ("GET", "/api/master/overrides"),
            ("DELETE", "/api/master/coals/临北/CSR"),
        ] {
            let response = test_app()
                .oneshot(session_request(
                    method,
                    uri,
                    "admin-session-token-0123456789abcdef",
                    Body::empty(),
                ))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{method} {uri}"
            );
        }
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

    // 密钥现在存在数据库里 (管理端生成/销毁), 所以"密钥有效"这条路径需要真库,
    // 留给部署后的手工验证。以下用例覆盖的是**不需要库也必须成立**的部分:
    // 缺密钥、会话冒充、畸形请求体不得绕过鉴权。

    /// 没带 X-API-Key 一律 401, 且必须发生在碰库之前.
    #[tokio::test]
    async fn test_data_api_rejects_missing_key() {
        let response = test_app()
            .oneshot(key_request(
                "POST",
                "/api/master/coals",
                None,
                Body::from(r#"{"updates":[]}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 用户会话 Cookie 不是这把钥匙的替代品.
    #[tokio::test]
    async fn test_session_cookie_is_not_an_api_key() {
        let response = test_app()
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

    /// 畸形请求体在**鉴权之前**不得被解析 —— 否则 axum 的 Json extractor 会把
    /// 接口 schema (字段名/类型) 回显给任何未鉴权访客。
    #[tokio::test]
    async fn test_malformed_body_does_not_bypass_auth() {
        let router = test_app();

        for body in ["{not json", r#"{"updates":[{"coal":1}]}"#] {
            let response = router
                .clone()
                .oneshot(key_request(
                    "POST",
                    "/api/master/coals",
                    None,
                    Body::from(body),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "body={body}");
            let text = response_text(response).await;
            assert!(!text.contains("updates"), "不得回显字段名: {text}");
            assert!(!text.contains("invalid type"), "不得回显类型错误: {text}");
        }

        // 缺 Content-Type 同样先撞鉴权, 而不是 415
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

    /// 查询与回滚接口同样要密钥.
    #[tokio::test]
    async fn test_override_read_and_delete_need_key() {
        let router = test_app();
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

    /// 管理端密钥接口走登录会话, 未登录一律 401 —— 管钥匙的不能是钥匙自己.
    #[tokio::test]
    async fn test_admin_key_endpoints_require_login() {
        let router = test_app();
        let cases: [(&str, &str, Body); 3] = [
            ("GET", "/api/admin/keys", Body::empty()),
            (
                "POST",
                "/api/admin/keys",
                Body::from(r#"{"name":"friend"}"#),
            ),
            ("DELETE", "/api/admin/keys/some-id", Body::empty()),
        ];
        for (method, uri, body) in cases {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(body)
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }

        // 带上 X-API-Key 也不行: 数据密钥管不了密钥本身
        let response = test_app()
            .oneshot(key_request(
                "POST",
                "/api/admin/keys",
                Some("dk_0123456789abcdef0123456789abcdef"),
                Body::from(r#"{"name":"friend"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 管理员已登录但名字非法时 422, 且在碰库之前就判掉.
    #[tokio::test]
    async fn test_create_key_rejects_bad_name() {
        for name in ["", "   ", &"名".repeat(MAX_KEY_NAME_CHARS + 1)] {
            let body = serde_json::to_string(&json!({ "name": name })).unwrap();
            let response = test_app()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/admin/keys")
                        .header(
                            header::COOKIE,
                            "doudou_session=admin-session-token-0123456789abcdef",
                        )
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "name={name:?}"
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
