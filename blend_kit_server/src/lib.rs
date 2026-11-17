use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{ConnectInfo, Extension, FromRequestParts, Json, MatchedPath, Path, State},
    handler::Handler,
    http::{header, request::Parts, HeaderMap, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{on, MethodFilter, MethodRouter},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use tower_http::{
    compression::CompressionLayer,
    services::{fs::ServeFileSystemResponseBody, ServeDir, ServeFile},
    set_header::{SetResponseHeader, SetResponseHeaderLayer},
    trace::TraceLayer,
};

mod admin;
mod api_key;
mod benchmarks;
mod coal_index;
mod coal_prices;
mod database;
mod forecast;
mod limits;
mod master_data;
mod mcci;
mod mcp;
mod quotes;
mod today;

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
/// 普通会话令牌的建议最短字节数. 只告警不拒绝: 线上令牌长度未知, 拒绝可能让部署起不来.
const MIN_SESSION_TOKEN_BYTES: usize = 32;
/// 登录失败后的固定等待, 拖慢在线暴力猜测 (按来源限速见 `limits::LoginGuard`).
const LOGIN_FAILURE_DELAY: Duration = Duration::from_secs(1);
/// MCP 一个请求里最多的 JSON-RPC 消息条数; 每条 tools/call 都要求解一次.
const MAX_MCP_MESSAGES: usize = 16;

/// 线上入口. `auth` 由调用方先建好 ([`AuthConfig::from_env`]): 监听哪个地址要看它
/// ([`AuthConfig::listen_ip`]), 得在绑定端口之前知道. 数据库配了却连不上、迁移失败时返回 Err,
/// 调用方记下原因后退出.
pub async fn app(public_dir: PathBuf, auth: AuthConfig) -> Result<Router, String> {
    let database = database::connect_from_env().await?;
    // k 启动时读一次, 之后只由 PUT /api/admin/settings 更新 —— 求解路径不碰库.
    let rank_k = match database.as_ref() {
        Some(pool) => admin::load_rank_interaction_k(pool).await,
        None => None,
    };
    if let Some(pool) = database.clone() {
        let master: serde_json::Value =
            serde_json::from_str(blend_kit::master_json()).unwrap_or_default();
        mcci::spawn_daily(pool.clone(), &master);
        coal_prices::spawn_hourly(pool);
    }
    Ok(app_with_state(
        public_dir,
        AppState {
            auth,
            database,
            rank_k: Arc::new(RwLock::new(rank_k)),
        },
    ))
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
    /// 登录限速状态, 随这份配置共享.
    login_guard: Arc<limits::LoginGuard>,
    /// 账号、口令、会话令牌里有没显式配置、退回了内置默认值的 (只该出现在本机开发).
    /// 为真时只监听本机回环, 见 [`AuthConfig::listen_ip`].
    default_credentials: bool,
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
            login_guard: Arc::default(),
            default_credentials: false,
        }
    }

    /// 监听哪个地址. 用了内置默认口令 (谁都知道) 就只听本机回环, 免得同一个镜像在 Railway 之外
    /// 没配 `AUTH_*` 就启动时带着默认口令对外; 三项都显式配置了才对外监听.
    pub fn listen_ip(&self) -> IpAddr {
        if self.default_credentials {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        }
    }

    /// 是否用了内置默认口令 (启动日志据此说明为什么只监听本机).
    pub fn uses_default_credentials(&self) -> bool {
        self.default_credentials
    }

    fn short_session_token(&self) -> bool {
        self.session_token.len() < MIN_SESSION_TOKEN_BYTES
    }

    /// 打开或关闭按来源 IP 的登录锁定 (默认关). 全站并发名额与失败等待不受影响.
    fn with_login_ip_lockout(mut self, enabled: bool) -> Self {
        self.login_guard = Arc::new(limits::LoginGuard::with_ip_lockout(enabled));
        self
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

    /// 从环境变量读配置. Railway 上缺 `AUTH_*` 返回 Err (启动失败), 本机缺了退回内置默认口令.
    /// 设成空串或只有空白的变量按"没设"处理: 空令牌等于谁都能登录.
    pub fn from_env() -> Result<Self, String> {
        let on_railway = std::env::var_os("RAILWAY_ENVIRONMENT_NAME").is_some();
        Self::from_lookup(on_railway, |name| std::env::var(name).ok())
    }

    /// [`AuthConfig::from_env`] 的本体; `lookup` 按名字取环境变量, 单测时换成固定值.
    fn from_lookup(
        on_railway: bool,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        // 变量在而值是空的 (部署配置里留了个空项) 不算配置过: 否则空的会话令牌会和空 Cookie 对上.
        let lookup = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
        let mut default_credentials = false;
        let mut read = |name: &str, local_default: &str| match lookup(name) {
            Some(value) => Ok(value),
            None if on_railway => Err(format!("Railway 部署必须设置 {name}")),
            None => {
                default_credentials = true;
                Ok(local_default.to_string())
            }
        };
        let username = read("AUTH_USERNAME", "doudou")?;
        let password = read("AUTH_PASSWORD", "123456")?;
        let session_token = read("AUTH_SESSION_TOKEN", "local-development-session")?;

        let ip_lockout = limits::ip_lockout_enabled(lookup(limits::IP_LOCKOUT_ENV).as_deref());
        tracing::info!(
            ip_lockout,
            "登录按来源 IP 锁定: {} ({} 设为 on 打开)",
            if ip_lockout { "开" } else { "关" },
            limits::IP_LOCKOUT_ENV
        );
        let mut config = Self::new(username, password, session_token, on_railway)
            .with_login_ip_lockout(ip_lockout);
        config.default_credentials = default_credentials;
        // 本机默认令牌不提醒; 显式配置的短令牌只告警, 不拒绝启动.
        if lookup("AUTH_SESSION_TOKEN").is_some() && config.short_session_token() {
            tracing::warn!(
                "AUTH_SESSION_TOKEN 短于 {MIN_SESSION_TOKEN_BYTES} 字节, 可被穷举, 请换成更长的随机值"
            );
        }
        // 管理员三项全部可选: 缺任一项即停用管理端, Railway 上也照常启动.
        Ok(
            match (
                lookup("ADMIN_USERNAME"),
                lookup("ADMIN_PASSWORD"),
                lookup("ADMIN_SESSION_TOKEN"),
            ) {
                (Some(username), Some(password), Some(token)) => {
                    config.with_admin(username, password, token)
                }
                _ => {
                    tracing::info!("ADMIN_* 未配齐, 管理端停用");
                    config
                }
            },
        )
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
    let https_only = state.auth.secure_cookie;
    // 不发 Cache-Control 时浏览器按 Last-Modified 自行估算缓存时长, 部署后几小时还在跑旧前端.
    // 打包产物文件名带内容哈希, 成功的可以永久缓存; 页面每次向服务器确认 (没变就回 304).
    let assets = SetResponseHeader::overriding(
        ServeDir::new(public_dir.join("assets")),
        header::CACHE_CONTROL,
        |response: &Response<ServeFileSystemResponseBody>| {
            let cache = HeaderValue::from_static("public, max-age=31536000, immutable");
            response.status().is_success().then_some(cache)
        },
    );
    let index_file = public_dir.join("index.html");
    let static_files = SetResponseHeader::overriding(
        ServeDir::new(public_dir)
            .append_index_html_on_directories(true)
            .fallback(ServeFile::new(index_file)),
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );

    let router = api_router(&state)
        .nest_service("/assets", assets)
        .fallback_service(static_files)
        .with_state(state)
        .layer(CompressionLayer::new());
    with_security_headers(router, https_only)
        .layer(TraceLayer::new_for_http().make_span_with(trace_span))
}

/// 访问日志的 span: 只记方法与路由模板 (如 `/mcp/{key}`), 不记实际路径. `tower_http` 默认的
/// span 带完整 URI, 日志一旦开到 debug, `/mcp/{key}` 路径里的密钥就进了日志. 没匹配上路由表的
/// 请求 (静态资源、SPA 兜底、写错的地址) 记 `-` —— 写错的地址里同样可能带着密钥.
/// 验证见 `test_access_log_never_contains_the_key_in_the_path`.
fn trace_span(request: &Request<Body>) -> tracing::Span {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("-", MatchedPath::as_str);
    tracing::debug_span!("request", method = %request.method(), route, version = ?request.version())
}

// ---------------------------------------------------------------------------
// 路由表与鉴权
//
// 每条路由都登记在 `routes()` 里, 并写明谁能进 (`Access`). 鉴权由这一类的中间件在进 handler
// 之前做完 —— 也就在请求体解析之前, 所以未登录的请求拿不到 422 里的字段名, 也触发不了
// 反序列化. handler 不再自己验; 要用身份就收一个 `Authed<…>` 参数. 路由表测试
// (`test_every_route_without_credentials_gets_401`) 照这张表逐条不带凭据去请求, 新加的路由
// 自动在内; 路由只能经这张表挂上去 (`test_routes_are_only_mounted_through_the_table`).
// ---------------------------------------------------------------------------

/// 一条路由谁能进. 新路由必须选一种.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    /// 不用登录.
    Public,
    /// 登录会话 (普通账号; 管理员会话是它的超集).
    Session,
    /// 管理员会话. 不认 X-API-Key —— 管钥匙的不能是钥匙自己.
    Admin,
    /// 煤库数据更新: 管理员会话, 或 data 用途的机器密钥 (X-API-Key). 普通用户会话无权.
    DataWriter,
    /// MCP (手机 Agent): mcp 用途的机器密钥, 放在请求头或路径里.
    McpKey,
}

impl Access {
    /// 给一条路由套上这一类的鉴权. 用 `MethodRouter::route_layer`: 只包住真正处理请求的 handler,
    /// 路径在而方法不对的请求照旧回 405 (不先问凭据). 穷尽 `match`: 新加一类不写鉴权就编不过.
    fn guard(self, handler: MethodRouter<AppState>, state: &AppState) -> MethodRouter<AppState> {
        let state = state.clone();
        match self {
            Access::Public => handler,
            Access::Session => {
                handler.route_layer(middleware::from_fn_with_state(state, require_session))
            }
            Access::Admin => {
                handler.route_layer(middleware::from_fn_with_state(state, require_admin))
            }
            Access::DataWriter => {
                handler.route_layer(middleware::from_fn_with_state(state, require_data_writer))
            }
            Access::McpKey => {
                handler.route_layer(middleware::from_fn_with_state(state, require_mcp_key))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Verb {
    fn filter(self) -> MethodFilter {
        match self {
            Verb::Get => MethodFilter::GET,
            Verb::Post => MethodFilter::POST,
            Verb::Put => MethodFilter::PUT,
            Verb::Patch => MethodFilter::PATCH,
            Verb::Delete => MethodFilter::DELETE,
        }
    }

    #[cfg(test)]
    fn method(self) -> axum::http::Method {
        match self {
            Verb::Get => axum::http::Method::GET,
            Verb::Post => axum::http::Method::POST,
            Verb::Put => axum::http::Method::PUT,
            Verb::Patch => axum::http::Method::PATCH,
            Verb::Delete => axum::http::Method::DELETE,
        }
    }
}

/// 路由表的一行.
struct RouteDef {
    access: Access,
    /// 只给路由表测试看: 挂路由用的是 `handler`, 它已经按这个方法建好.
    #[cfg(test)]
    verb: Verb,
    path: &'static str,
    handler: MethodRouter<AppState>,
}

fn route<H, T>(access: Access, verb: Verb, path: &'static str, handler: H) -> RouteDef
where
    H: Handler<T, AppState>,
    T: 'static,
{
    RouteDef {
        access,
        #[cfg(test)]
        verb,
        path,
        handler: on(verb.filter(), handler),
    }
}

/// 全部路由 (静态资源与 SPA 兜底除外).
fn routes() -> Vec<RouteDef> {
    use Access::{Admin, DataWriter, McpKey, Public, Session};
    use Verb::{Delete, Get, Patch, Post, Put};
    vec![
        route(Public, Get, "/api/health", health),
        route(Public, Get, "/api/coal-index", coal_index_handler),
        route(Public, Get, "/api/auth/session", auth_session),
        route(Public, Post, "/api/auth/login", login),
        route(Public, Post, "/api/auth/logout", logout),
        // 本服务不开 SSE 推送流, GET 按规范回 405.
        route(Public, Get, "/mcp", mcp_get),
        route(Public, Get, "/mcp/{key}", mcp_get),
        route(Session, Post, "/api/solve", solve),
        route(Session, Post, "/api/price", benchmarks::estimate),
        route(Session, Get, "/api/coal-prices", coal_prices::list),
        route(
            Session,
            Get,
            "/api/coal-prices/history",
            coal_prices::history,
        ),
        route(Session, Post, "/api/quotes", quotes::archive),
        route(Session, Get, "/api/quotes/latest", quotes::latest),
        route(Session, Get, "/api/benchmarks", benchmarks::list),
        route(Session, Get, "/api/mcci", mcci::list),
        route(Session, Get, "/api/master", master),
        route(Session, Get, "/api/storage", get_storage),
        route(Session, Put, "/api/storage", put_storage),
        route(Session, Get, "/api/history", list_history),
        route(Session, Post, "/api/history", create_history),
        route(Session, Delete, "/api/history", clear_history),
        route(Session, Get, "/api/history/count", count_history),
        route(Session, Post, "/api/history/import", import_history),
        route(
            Session,
            Patch,
            "/api/history/{id}/measured",
            set_measured_quality,
        ),
        route(Admin, Get, "/api/admin/quotes", quotes::list),
        route(Admin, Delete, "/api/admin/quotes/{id}", quotes::delete),
        route(Admin, Post, "/api/admin/benchmarks", benchmarks::create),
        route(
            Admin,
            Delete,
            "/api/admin/benchmarks/{id}",
            benchmarks::delete,
        ),
        route(Admin, Get, "/api/admin/keys", list_api_keys),
        route(Admin, Post, "/api/admin/keys", create_api_key),
        route(Admin, Delete, "/api/admin/keys/{id}", revoke_api_key),
        route(Admin, Get, "/api/admin/settings", admin::get_settings),
        route(Admin, Put, "/api/admin/settings", admin::put_settings),
        route(Admin, Get, "/api/admin/calibration", admin::calibration),
        route(Admin, Get, "/api/admin/csr-model", admin::csr_model),
        route(DataWriter, Post, "/api/master/coals", update_coal_data),
        route(
            DataWriter,
            Get,
            "/api/master/overrides",
            list_coal_overrides,
        ),
        route(
            DataWriter,
            Delete,
            "/api/master/coals/{coal}/{field}",
            delete_coal_override,
        ),
        route(McpKey, Post, "/mcp", mcp_post),
        route(McpKey, Post, "/mcp/{key}", mcp_post),
    ]
}

/// 按路由表建路由器: 每条路由先套上它那一类的鉴权中间件再挂上去 (同一路径的不同方法可以
/// 各是一类, 如 `/mcp` 的 GET 公开、POST 要密钥).
fn api_router(state: &AppState) -> Router<AppState> {
    routes().into_iter().fold(Router::new(), |router, def| {
        router.route(def.path, def.access.guard(def.handler, state))
    })
}

/// 登录会话的身份 (由 [`require_session`] 放进请求扩展). 线上所有登录者共用一个网页账号,
/// 管理员会话读写的也是这一行数据, 所以只有用户名.
#[derive(Clone)]
struct SessionUser {
    username: String,
}

/// 管理员的身份 (由 [`require_admin`] 放进请求扩展), 留痕用.
#[derive(Clone)]
struct AdminUser {
    username: String,
}

/// 煤库数据更新的调用方 (由 [`require_data_writer`] 放进请求扩展).
#[derive(Clone)]
struct DataWriter {
    /// 落库留痕用的持有者名字: 管理员为 `管理员:<账号>`, 密钥为密钥名字 (调用方伪造不了).
    holder: String,
    /// 走的是 Cookie 会话 (写接口据此要求 JSON Content-Type).
    by_session: bool,
}

/// 从请求扩展里取鉴权中间件放进去的身份. 取不到说明这条路由没挂在对应的组里 ——
/// 当作没登录回 401, 不放行.
struct Authed<T>(T);

impl<S, T> FromRequestParts<S> for Authed<T>
where
    S: Send + Sync,
    T: Clone + Send + Sync + 'static,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<T>() {
            Some(identity) => Ok(Self(identity.clone())),
            None => {
                tracing::error!("handler 要身份, 但这条路由没挂在对应的鉴权组里");
                Err(unauthorized())
            }
        }
    }
}

/// 登录会话组: 没有有效会话 401.
async fn require_session(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if session_role(request.headers(), &state.auth).is_none() {
        return unauthorized();
    }
    request.extensions_mut().insert(SessionUser {
        username: state.auth.username.clone(),
    });
    next.run(request).await
}

/// 管理员组: 未登录 401, 普通账号 403.
async fn require_admin(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match (
        session_role(request.headers(), &state.auth),
        &state.auth.admin,
    ) {
        (Some(Role::Admin), Some(admin)) => {
            request.extensions_mut().insert(AdminUser {
                username: admin.username.clone(),
            });
            next.run(request).await
        }
        (None, _) => unauthorized(),
        _ => (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            r#"{"ok":false,"reason":"需要管理员权限"}"#,
        )
            .into_response(),
    }
}

/// 煤库数据更新组: 管理员会话或 data 用途的 X-API-Key 二选一.
async fn require_data_writer(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match check_data_writer(request.headers(), &state).await {
        Ok(writer) => {
            request.extensions_mut().insert(writer);
            next.run(request).await
        }
        Err(response) => *response,
    }
}

/// MCP 组: mcp 用途的密钥, 取自路径 (`/mcp/{key}`) 或请求头. 验过才往下走, 请求体留给 handler 读.
async fn require_mcp_key(
    State(state): State<AppState>,
    key: Option<Path<String>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let presented = match key {
        Some(Path(key)) => key,
        None => mcp_key_from_headers(request.headers()),
    };
    match check_mcp_key(&state, &presented).await {
        Ok(()) => next.run(request).await,
        Err(response) => *response,
    }
}

/// 安全响应头. 不加限制脚本/样式来源的 CSP, 只用 frame-ancestors 禁止被别的站点嵌入.
/// HSTS 只在 Railway (secure_cookie 为真, 一定走 HTTPS) 上发, 本机 http 调试不受影响.
fn with_security_headers(router: Router, https_only: bool) -> Router {
    let set =
        |name, value| SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value));
    let router = router
        .layer(set(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(set(header::X_FRAME_OPTIONS, "SAMEORIGIN"))
        .layer(set(
            header::CONTENT_SECURITY_POLICY,
            "frame-ancestors 'self'",
        ))
        .layer(set(header::REFERRER_POLICY, "same-origin"));
    if https_only {
        router.layer(set(header::STRICT_TRANSPORT_SECURITY, "max-age=31536000"))
    } else {
        router
    }
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

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(input): Json<LoginRequest>,
) -> Response {
    let auth = &state.auth;
    let guard = &auth.login_guard;
    let peer = peer.map(|Extension(ConnectInfo(addr))| addr);
    let source = limits::client_ip(&headers, peer);
    let locked = || {
        guard.retry_after(&source, Instant::now()).map(|wait| {
            let reason = format!("登录失败次数过多, 请 {} 秒后再试", wait.as_secs().max(1));
            limits::too_many_logins(wait, &reason)
        })
    };
    if let Some(response) = locked() {
        return response;
    }
    // 名额一直占到失败等待结束, 并发猜密码的总速率因此有上限.
    let Some(_slot) = guard.slot().await else {
        return limits::too_many_logins(Duration::from_secs(5), "登录请求过多, 请稍后再试");
    };
    if let Some(response) = locked() {
        return response;
    }
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
            // 记原始代理头与对端地址 (不记账号密码), 上线后据此判断代理是覆盖还是追加.
            tracing::warn!(
                x_forwarded_for = %limits::header_for_log(&headers, "x-forwarded-for"),
                x_real_ip = %limits::header_for_log(&headers, "x-real-ip"),
                peer = %peer.map_or_else(|| "-".to_string(), |addr| addr.to_string()),
                %source,
                "登录失败"
            );
            guard.record_failure(&source, Instant::now());
            tokio::time::sleep(LOGIN_FAILURE_DELAY).await;
            return (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
                r#"{"authenticated":false,"reason":"账号或密码错误"}"#,
            )
                .into_response();
        }
    };
    guard.record_success(&source);

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

async fn solve(State(state): State<AppState>, request: Request<Body>) -> Response {
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
    let input = admin::inject_rank_interaction(input, state.rank_k()).into_owned();
    // 求解是纯 CPU 计算 (带价格稳健性分析时要改价重解上百次), 挪出异步线程, 免得卡住
    // 同一 worker 上的其他请求; 并发与时长受 limits 约束.
    let output = limits::run_solver(move || blend_kit::solve_json(&input))
        .await
        .unwrap_or_else(|error| json!({ "ok": false, "reason": error.reason("求解") }).to_string());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        output,
    )
        .into_response()
}

async fn master(State(state): State<AppState>) -> Response {
    (
        [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        merged_master_json(&state).await,
    )
        .into_response()
}

/// 基线 (编译期嵌入, 进 git) + 覆盖层 (PG, 由数据更新接口写入).
/// 没连库或读失败时退回纯基线 —— 煤库能少几个补充指标, 但不能整个不可用.
async fn merged_master_json(state: &AppState) -> std::borrow::Cow<'static, str> {
    merged_master_json_from_pool(state.database.as_ref())
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "读取或合并煤库覆盖层失败, 退回基线");
            blend_kit::master_json().into()
        })
}

/// 快照生成必须知道读库是否成功, 不能把降级基线写成当天的全量快照.
async fn merged_master_json_from_pool(
    pool: Option<&PgPool>,
) -> Result<std::borrow::Cow<'static, str>, String> {
    if let Some(pool) = pool {
        let overrides = database::list_coal_overrides(pool)
            .await
            .map_err(|error| error.to_string())?;
        if !overrides.is_empty() {
            return master_data::merge(&overrides).map(Into::into);
        }
    }
    Ok(blend_kit::master_json().into())
}

async fn get_storage(State(state): State<AppState>, Authed(user): Authed<SessionUser>) -> Response {
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::load_storage(pool, &user.username).await {
        Ok(storage) => Json(storage).into_response(),
        Err(error) => database_error(error),
    }
}

async fn put_storage(
    State(state): State<AppState>,
    Authed(user): Authed<SessionUser>,
    Json(mut storage): Json<database::UserStorage>,
) -> Response {
    if let Err(reason) = storage.validate() {
        return bad_request(reason);
    }
    storage.initialized = true;
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::save_storage(pool, &user.username, &storage).await {
        Ok(()) => Json(storage).into_response(),
        Err(error) => database_error(error),
    }
}

async fn create_history(
    State(state): State<AppState>,
    Authed(user): Authed<SessionUser>,
    Json(input): Json<database::CreateHistory>,
) -> Response {
    if let Err(reason) = input.validate() {
        return bad_request(reason);
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::create_history(pool, &user.username, &input).await {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(error) => database_error(error),
    }
}

async fn import_history(
    State(state): State<AppState>,
    Authed(user): Authed<SessionUser>,
    Json(input): Json<database::ImportHistory>,
) -> Response {
    if input.entries.len() > 100 {
        return bad_request("一次最多导入 100 条历史方案");
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::import_history(pool, &user.username, &input).await {
        Ok(imported) => Json(json!({ "imported": imported })).into_response(),
        Err(error) => database_error(error),
    }
}

async fn count_history(
    State(state): State<AppState>,
    Authed(user): Authed<SessionUser>,
) -> Response {
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::count_history(pool, &user.username).await {
        Ok(count) => Json(database::count_response(count)).into_response(),
        Err(error) => database_error(error),
    }
}

async fn list_history(
    State(state): State<AppState>,
    Authed(user): Authed<SessionUser>,
) -> Response {
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::list_history(pool, &user.username).await {
        Ok(history) => Json(history).into_response(),
        Err(error) => database_error(error),
    }
}

async fn set_measured_quality(
    State(state): State<AppState>,
    Authed(user): Authed<SessionUser>,
    Path(id): Path<String>,
    Json(measured): Json<database::MeasuredQuality>,
) -> Response {
    if let Err(reason) = measured.validate() {
        return bad_request(reason);
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::set_measured_quality(pool, &user.username, &id, &measured).await {
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

async fn clear_history(
    State(state): State<AppState>,
    Authed(user): Authed<SessionUser>,
) -> Response {
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match database::clear_history(pool, &user.username).await {
        Ok(deleted) => Json(json!({ "deleted": deleted })).into_response(),
        Err(error) => database_error(error),
    }
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
    // 空令牌永远不算登录: 即使配置里的令牌被弄成了空串, 也不能让空 Cookie 对上.
    if token.is_empty() {
        return None;
    }
    if auth
        .admin
        .as_ref()
        .is_some_and(|admin| secret_eq(token, &admin.session_token))
    {
        return Some(Role::Admin);
    }
    secret_eq(token, &auth.session_token).then_some(Role::User)
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

/// 数据更新接口的鉴权: 管理员会话或有效 X-API-Key (data 用途) 二选一.
async fn check_data_writer(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<DataWriter, Box<Response>> {
    if let (Some(Role::Admin), Some(admin)) =
        (session_role(headers, &state.auth), &state.auth.admin)
    {
        return Ok(DataWriter {
            holder: format!("管理员:{}", admin.username),
            by_session: true,
        });
    }
    let presented = headers
        .get(DATA_API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let rejected = || {
        // 只记事件, 不记 presented —— 那是攻击者可控内容, 不该进日志.
        tracing::warn!("数据更新接口密钥校验失败");
        (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            r#"{"ok":false,"reason":"X-API-Key 无效或已销毁"}"#,
        )
            .into_response()
    };
    let holder = check_api_key(state, presented, api_key::Scope::Data, rejected).await?;
    Ok(DataWriter {
        holder,
        by_session: false,
    })
}

/// 验一把机器密钥: 对来访明文做 SHA-256, 查未销毁、用途为 `scope` 的密钥行 —— 用途不符
/// 与密钥不存在一样对不上. 成功返回密钥的名字 (落库留痕用, 调用方伪造不了); 没带或对不上
/// 回 `rejected` 给的 401 (发生在碰库之前), 没配数据库 503.
async fn check_api_key(
    state: &AppState,
    presented: &str,
    scope: api_key::Scope,
    rejected: impl Fn() -> Response,
) -> Result<String, Box<Response>> {
    if presented.is_empty() {
        return Err(Box::new(rejected()));
    }
    let pool = require_database(state)?;
    match database::authenticate_api_key(pool, &api_key::hash_key(presented), scope).await {
        Ok(Some(name)) => Ok(name),
        Ok(None) => Err(Box::new(rejected())),
        Err(error) => Err(Box::new(database_error(error))),
    }
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

// ---------------------------------------------------------------------------
// 管理端: 密钥的生成 / 列出 / 销毁
//
// 这三个接口走**管理员会话**(登录 Cookie), 不是 X-API-Key —— 管钥匙的不能是钥匙
// 自己, 否则一把泄露的密钥就能给自己发新的。
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateKeyRequest {
    name: String,
    /// 用途: data (写煤库, 默认) 或 mcp (手机 Agent 查询 + 求解 + 询价存档).
    #[serde(default = "default_key_scope")]
    scope: String,
}

fn default_key_scope() -> String {
    "data".into()
}

/// POST /api/admin/keys —— 生成一把新密钥. 明文只在这一次响应里出现.
async fn create_api_key(
    State(state): State<AppState>,
    Json(payload): Json<CreateKeyRequest>,
) -> Response {
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
    let Some(scope) = api_key::Scope::parse(&payload.scope) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "ok": false, "reason": "scope 只能是 data 或 mcp" })),
        )
            .into_response();
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };

    let generated = api_key::generate();
    match database::create_api_key(pool, name, scope, &generated.hash, &generated.prefix).await {
        Ok(row) => {
            tracing::info!(name, scope = scope.as_str(), "已生成密钥");
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
async fn list_api_keys(State(state): State<AppState>) -> Response {
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
async fn revoke_api_key(State(state): State<AppState>, Path(id): Path<String>) -> Response {
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
/// 这里收的是 `Request<Body>` 而不是 `Json<T>`: 请求体不合法时回的是本接口自己的 400 与说明,
/// 不是 axum 的 422; 鉴权已由 [`require_data_writer`] 在进来之前做完.
async fn update_coal_data(
    Authed(writer): Authed<DataWriter>,
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    // 持有者名字由密钥推导, 不接受调用方自报 —— 出了坏数据要查得到人.
    if writer.by_session {
        if let Err(response) = require_json_content_type(&headers) {
            return *response;
        }
    }
    let holder = writer.holder;

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
async fn list_coal_overrides(State(state): State<AppState>) -> Response {
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
    Path((coal, field)): Path<(String, String)>,
) -> Response {
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

// ---------------------------------------------------------------------------
// MCP 入口 (手机 Agent): 用途为 mcp 的密钥, 查询 + 求解 + 询价存档, 写不了煤库
// ---------------------------------------------------------------------------

/// 从请求头取 MCP 密钥: `Authorization: Bearer <key>` 或 `X-API-Key`.
fn mcp_key_from_headers(headers: &HeaderMap) -> String {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    let api_key = headers
        .get(DATA_API_KEY_HEADER)
        .and_then(|value| value.to_str().ok());
    bearer.or(api_key).unwrap_or_default().to_string()
}

/// 验 MCP 密钥 (mcp 用途). data 用途的写煤库密钥进不来.
async fn check_mcp_key(state: &AppState, presented: &str) -> Result<(), Box<Response>> {
    let rejected = || {
        tracing::warn!("MCP 密钥校验失败");
        (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            r#"{"ok":false,"reason":"MCP 密钥无效或已销毁 (须为管理端生成的 mcp 用途密钥)"}"#,
        )
            .into_response()
    };
    check_api_key(state, presented, api_key::Scope::Mcp, rejected)
        .await
        .map(drop)
}

/// GET /mcp —— 本服务不开 SSE 推送流, 按规范回 405.
async fn mcp_get() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

/// POST /mcp 与 POST /mcp/{key}. 密钥已由 [`require_mcp_key`] 验过 (先验密钥, 再读请求体);
/// 后一个地址给只能填一个地址、设不了请求头的客户端 —— 访问日志只记路由模板, 密钥不进日志
/// (见 `trace_span`), 泄露时在管理端销毁即可.
async fn mcp_post(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    // 下面的存档闭包要按引用带着它走.
    let state = &state;
    let pool = match require_database(state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Json(json!({
            "jsonrpc": "2.0", "id": null,
            "error": { "code": -32700, "message": "请求体不是合法 JSON" }
        }))
        .into_response();
    };
    let (messages, batch) = match mcp_messages(parsed) {
        Ok(split) => split,
        Err(error) => return Json(error).into_response(),
    };
    let context = if messages.iter().any(mcp::needs_context) {
        match load_mcp_context(state, pool, &messages).await {
            Ok(context) => Some(context),
            Err(error) => return database_error(error),
        }
    } else {
        None
    };
    let store = |day, batch: quotes::QuoteBatch| async move {
        quotes::store(state, &state.auth.username, &batch, day)
            .await
            .map(drop)
    };
    let mut replies = mcp_answer(messages, context, limits::run_solver, store).await;
    match (replies.is_empty(), batch) {
        // 全是通知: 规范要求 202, 不带内容
        (true, _) => StatusCode::ACCEPTED.into_response(),
        (false, true) => Json(serde_json::Value::Array(replies)).into_response(),
        (false, false) => Json(replies.remove(0)).into_response(),
    }
}

/// 拆出 JSON-RPC 消息与是否批量; 批量超过 `MAX_MCP_MESSAGES` 条回 JSON-RPC 错误.
fn mcp_messages(
    parsed: serde_json::Value,
) -> Result<(Vec<serde_json::Value>, bool), serde_json::Value> {
    match parsed {
        serde_json::Value::Array(list) if list.len() > MAX_MCP_MESSAGES => Err(json!({
            "jsonrpc": "2.0", "id": null,
            "error": {
                "code": -32600,
                "message": format!("一个请求最多 {MAX_MCP_MESSAGES} 条消息, 收到 {} 条", list.len())
            }
        })),
        serde_json::Value::Array(list) => Ok((list, true)),
        single => Ok((vec![single], false)),
    }
}

/// 一批 MCP 消息里要在求解闸门里算的那部分 (带原位置), 交给 `run` 执行.
type McpTask = Box<dyn FnOnce() -> Vec<(usize, serde_json::Value)> + Send>;

/// 要过求解闸门的消息: 求解或估价的 `tools/call` (哪些工具算, 见 [`mcp::Tool::computes`]).
/// 工具名认不出的调用照旧也走闸门 (回的是 "没有这个工具").
fn mcp_computes(message: &serde_json::Value) -> bool {
    mcp::needs_context(message) && mcp::Tool::called_by(message).is_none_or(mcp::Tool::computes)
}

/// 回一批 MCP 消息, 并给其中算成了的询价精算存档 (`store`: 线上为 [`quotes::store`], 与
/// `POST /api/quotes` 同一条路, 记在网页账号名下). 要存的报价先由纯函数算出来, 回包算完才落库:
/// 存档不影响精算结果, 没求解成功的调用也不会留下存档.
async fn mcp_answer<Run, Solving, Store, Storing>(
    messages: Vec<serde_json::Value>,
    context: Option<mcp::Context>,
    run: Run,
    store: Store,
) -> Vec<serde_json::Value>
where
    Run: FnOnce(McpTask) -> Solving,
    Solving:
        std::future::Future<Output = Result<Vec<(usize, serde_json::Value)>, limits::ComputeError>>,
    Store: FnOnce(chrono::NaiveDate, quotes::QuoteBatch) -> Storing,
    Storing: std::future::Future<Output = Result<(), quotes::StoreError>>,
{
    let archives = mcp_quote_archives(&messages, context.as_ref());
    let mut replies = mcp_replies(messages, context, run).await;
    archive_mcp_quotes(archives, &mut replies, store).await;
    replies
}

/// 一条询价精算消息要存档的那批报价.
struct McpQuoteArchive {
    /// 这条消息的回包在回包列表里的位置 (不带 id 的通知不回包, 不占位置).
    reply_at: usize,
    id: serde_json::Value,
    /// 算批号用的那一天 (北京时间); 落库的 `quoted_on` 也用它, 跨零点的调用才不会批号是前一天、
    /// 日期是后一天.
    day: chrono::NaiveDate,
    batch: quotes::QuoteBatch,
}

/// 这批消息里各条询价精算要存档的报价; 没加载数据 (没有 tools/call) 为空.
fn mcp_quote_archives(
    messages: &[serde_json::Value],
    context: Option<&mcp::Context>,
) -> Vec<McpQuoteArchive> {
    let Some(context) = context else {
        return Vec::new();
    };
    let answered = messages
        .iter()
        .filter(|message| message.get("id").is_some_and(|id| !id.is_null()));
    answered
        .enumerate()
        .filter_map(|(reply_at, message)| {
            Some(McpQuoteArchive {
                reply_at,
                id: message["id"].clone(),
                day: context.today,
                batch: mcp::quote_archive(message, context)?,
            })
        })
        .collect()
}

const MCP_QUOTES_ARCHIVED: &str = "这批报价已按批存档, 供后台「询价对比」校准估价。";
const MCP_QUOTES_NOT_LAST: &str = "这批报价没有存档: 同一请求里有多条询价精算, 只存档最后一条。";

/// 哪一批真去存档. 只有回包是工具结果、且工具没报错的询价精算才算求解过: 求解闸门拒绝 (忙、超时)
/// 回的是 JSON-RPC error, 参数没过校验回的是 `isError`. 一个请求最多存一批 (不然批量消息能绕过
/// 求解闸门成批写库): 留最后一条, 其余的另列, 好在各自的回包里说明.
fn mcp_archives_to_store(
    archives: Vec<McpQuoteArchive>,
    replies: &[serde_json::Value],
) -> (Option<McpQuoteArchive>, Vec<McpQuoteArchive>) {
    let mut solved: Vec<McpQuoteArchive> = archives
        .into_iter()
        .filter(|archive| {
            replies.get(archive.reply_at).is_some_and(|reply| {
                reply["id"] == archive.id && reply["result"]["isError"] == false
            })
        })
        .collect();
    let last = solved.pop();
    (last, solved)
}

/// 往工具回包的文本结尾补一行; 不是带文本的工具结果 (如 JSON-RPC error) 原样返回.
fn mcp_reply_with_line(mut reply: serde_json::Value, line: &str) -> serde_json::Value {
    if let Some(serde_json::Value::String(text)) = reply.pointer_mut("/result/content/0/text") {
        text.push('\n');
        text.push_str(line);
    }
    reply
}

/// 存档的结论, 写给读回包的人: 存成了才说存了, 没存成说明原因 (数据库报错的原文只进日志).
fn mcp_archive_line(outcome: &Result<(), quotes::StoreError>) -> String {
    match outcome {
        Ok(()) => MCP_QUOTES_ARCHIVED.to_string(),
        Err(error) => format!("这批报价没有存档成功 (不影响上面的结果): {}", error.brief()),
    }
}

/// 给算成了的询价精算存档, 并把结论补到对应回包的文本结尾: 落库之后才知道成败, 所以这句话
/// 不能由排版那一层提前写. 存不成不影响回包里的精算结果, 只多一行说明并记日志.
async fn archive_mcp_quotes<Store, Storing>(
    archives: Vec<McpQuoteArchive>,
    replies: &mut [serde_json::Value],
    store: Store,
) where
    Store: FnOnce(chrono::NaiveDate, quotes::QuoteBatch) -> Storing,
    Storing: std::future::Future<Output = Result<(), quotes::StoreError>>,
{
    let (last, skipped) = mcp_archives_to_store(archives, replies);
    let mut note = |at: usize, line: &str| {
        replies[at] = mcp_reply_with_line(std::mem::take(&mut replies[at]), line);
    };
    for archive in skipped {
        note(archive.reply_at, MCP_QUOTES_NOT_LAST);
    }
    let Some(archive) = last else {
        return;
    };
    let batch_id = archive.batch.batch_id.clone();
    let outcome = store(archive.day, archive.batch).await;
    if let Err(error) = &outcome {
        tracing::warn!(
            %batch_id,
            reason = %error.reason(),
            "MCP 询价存档失败, 不影响精算结果"
        );
    }
    note(archive.reply_at, &mcp_archive_line(&outcome));
}

/// 逐条回 MCP 消息, 按原顺序. 只有求解/估价的消息交给 `run` (线上为求解闸门: CPU 密集,
/// 挪出异步线程, 受同一并发上限与超时约束); 握手、列工具之类当场回, 忙时也不排队.
async fn mcp_replies<Run, Fut>(
    messages: Vec<serde_json::Value>,
    context: Option<mcp::Context>,
    run: Run,
) -> Vec<serde_json::Value>
where
    Run: FnOnce(McpTask) -> Fut,
    Fut:
        std::future::Future<Output = Result<Vec<(usize, serde_json::Value)>, limits::ComputeError>>,
{
    let (computing, quick): (Vec<_>, Vec<_>) = messages
        .into_iter()
        .enumerate()
        .partition(|(_, message)| mcp_computes(message));
    let mut replies: Vec<(usize, serde_json::Value)> = quick
        .iter()
        .filter_map(|(at, message)| {
            mcp::handle(message, context.as_ref()).map(|reply| (*at, reply))
        })
        .collect();
    if !computing.is_empty() {
        let (positions, ids): (Vec<usize>, Vec<serde_json::Value>) = computing
            .iter()
            .filter_map(|(at, message)| {
                let id = message.get("id").filter(|id| !id.is_null())?;
                Some((*at, id.clone()))
            })
            .unzip();
        let task: McpTask = Box::new(move || {
            computing
                .iter()
                .filter_map(|(at, message)| {
                    mcp::handle(message, context.as_ref()).map(|reply| (*at, reply))
                })
                .collect()
        });
        match run(task).await {
            Ok(done) => replies.extend(done),
            Err(error) => replies.extend(
                positions
                    .into_iter()
                    .zip(mcp_internal_errors(ids, &error.reason("求解"))),
            ),
        }
    }
    replies.sort_by_key(|(at, _)| *at);
    replies.into_iter().map(|(_, reply)| reply).collect()
}

/// 求解线程出错时, 每条带 id 的消息各回一条 -32603 (通知不回).
fn mcp_internal_errors(ids: Vec<serde_json::Value>, message: &str) -> Vec<serde_json::Value> {
    ids.into_iter()
        .map(|id| {
            json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32603, "message": message }
            })
        })
        .collect()
}

/// 求解要用的线上数据: 合并后的煤库、网页账号保存的合同与煤池、现货指数最新值、煤阶交互 k.
/// 估价快照与行情只在这批消息里有工具用得着时才读; 读不到不算错, 工具照旧输出或如实说取不到.
async fn load_mcp_context(
    state: &AppState,
    pool: &PgPool,
    messages: &[serde_json::Value],
) -> Result<mcp::Context, sqlx::Error> {
    let called: Vec<mcp::Tool> = messages.iter().filter_map(mcp::Tool::called_by).collect();
    let wanted = |needs: fn(mcp::Tool) -> bool| called.iter().copied().any(needs);
    let coal_prices = if wanted(mcp::Tool::needs_coal_prices) {
        coal_prices::latest_snapshot(pool)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "读取每日估价快照失败, find_coal 不带涨跌与预测");
                None
            })
    } else {
        None
    };
    let market = if wanted(mcp::Tool::needs_market) {
        mcci::stored_overview(pool)
            .await
            .map_err(|error| tracing::warn!(%error, "读取指数行情失败, market 工具取不到数据"))
            .ok()
    } else {
        None
    };
    let master: serde_json::Value =
        serde_json::from_str(&merged_master_json(state).await).unwrap_or_default();
    let storage = database::load_storage(pool, &state.auth.username).await?;
    let benchmarks = benchmarks::current(state, &master).await;
    // 没有任何基准报价时才退回现货指数 (那是指数点, 只能当相对尺度用).
    let market_price = if benchmarks.is_empty() {
        coal_index::get_spot_series()
            .await
            .and_then(|points| points.last().map(|point| point.price))
    } else {
        None
    };
    Ok(mcp::Context {
        master,
        storage,
        market_price,
        benchmarks,
        rank_k: state.rank_k(),
        coal_prices,
        market,
        today: mcci::beijing_today(),
    })
}

/// 源文件里的生产代码: 去掉文件末尾的测试模块 —— 从第一个顶格的 `#[cfg(test)]` + `mod`
/// (中间可以隔一行 `#[path = …]`) 起. 夹在中间的 `#[cfg(test)]` 函数与 impl 留着一起看.
#[cfg(test)]
fn production_source(source: &str) -> &str {
    const TEST_MODULES: [&str; 2] = ["\n#[cfg(test)]\nmod ", "\n#[cfg(test)]\n#[path = "];
    let end = TEST_MODULES
        .iter()
        .filter_map(|marker| source.find(marker))
        .min()
        .unwrap_or(source.len());
    &source[..end]
}

#[cfg(test)]
#[path = "reliability_tests.rs"]
mod reliability_tests;

#[cfg(test)]
mod test_db;

#[cfg(test)]
mod db_smoke_tests;

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

    #[tokio::test]
    async fn test_admin_quotes_guards() {
        for (method, uri) in [
            ("GET", "/api/admin/quotes?limit=abc"),
            ("DELETE", "/api/admin/quotes/x"),
        ] {
            for (token, expected) in [
                ("forged", StatusCode::UNAUTHORIZED),
                ("test-session-token", StatusCode::FORBIDDEN),
            ] {
                let response = test_app()
                    .oneshot(session_request(method, uri, token, Body::empty()))
                    .await
                    .unwrap();
                assert_eq!(response.status(), expected);
                let value: Value = serde_json::from_str(&response_text(response).await).unwrap();
                assert_eq!(value["ok"], false);
            }
        }
        for (method, uri, expected) in [
            (
                "GET",
                "/api/admin/quotes?limit=abc",
                StatusCode::BAD_REQUEST,
            ),
            ("GET", "/api/admin/quotes?limit=0", StatusCode::BAD_REQUEST),
            ("GET", "/api/admin/quotes?limit=-1", StatusCode::BAD_REQUEST),
            (
                "GET",
                "/api/admin/quotes?limit=1.5",
                StatusCode::BAD_REQUEST,
            ),
            (
                "GET",
                "/api/admin/quotes?limit=1001",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            ("GET", "/api/admin/quotes", StatusCode::SERVICE_UNAVAILABLE),
            (
                "DELETE",
                "/api/admin/quotes/x",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
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
            assert_eq!(response.status(), expected, "{uri}");
            let value: Value = serde_json::from_str(&response_text(response).await).unwrap();
            assert_eq!(value["ok"], false);
        }
    }

    #[tokio::test]
    async fn test_archive_quotes_checks_login_type_body_then_database() {
        let valid =
            r#"{"batch_id":"abc-1","items":[{"name":"测试煤","fob":2150,"props":{"S":0.6}}]}"#;
        let invalid_props =
            r#"{"batch_id":"abc-1","items":[{"name":"测试煤","fob":2150,"props":{"A":-5}}]}"#;
        for (token, json_type, body, expected) in [
            ("forged", false, "not json", StatusCode::UNAUTHORIZED),
            ("forged", true, valid, StatusCode::UNAUTHORIZED),
            (
                "test-session-token",
                false,
                valid,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
            (
                "test-session-token",
                true,
                "not json",
                StatusCode::BAD_REQUEST,
            ),
            ("test-session-token", true, "[]", StatusCode::BAD_REQUEST),
            (
                "test-session-token",
                true,
                r#"{"batch_id":"abc-1","items":[]}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "test-session-token",
                true,
                invalid_props,
                StatusCode::BAD_REQUEST,
            ),
            (
                "test-session-token",
                true,
                valid,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            let mut request = session_request("POST", "/api/quotes", token, Body::from(body));
            if !json_type {
                request.headers_mut().remove(header::CONTENT_TYPE);
            }
            let response = test_app().oneshot(request).await.unwrap();
            assert_eq!(response.status(), expected, "{token}, {body}");
            let value: Value = serde_json::from_str(&response_text(response).await).unwrap();
            assert_eq!(value["ok"], false);
        }
        let request = session_request(
            "POST",
            "/api/quotes",
            "test-session-token",
            Body::from(" ".repeat(256 * 1024 + 1)),
        );
        assert_eq!(
            test_app().oneshot(request).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn test_latest_quotes_requires_login_and_database() {
        for (token, expected) in [
            ("forged", StatusCode::UNAUTHORIZED),
            ("test-session-token", StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = test_app()
                .oneshot(session_request(
                    "GET",
                    "/api/quotes/latest",
                    token,
                    Body::empty(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            let value: Value = serde_json::from_str(&response_text(response).await).unwrap();
            assert_eq!(value["ok"], false);
        }
    }

    #[tokio::test]
    async fn test_coal_prices_requires_login_and_database() {
        for (token, expected) in [
            ("forged", StatusCode::UNAUTHORIZED),
            ("test-session-token", StatusCode::SERVICE_UNAVAILABLE),
            (
                "admin-session-token-0123456789abcdef",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            let response = test_app()
                .oneshot(session_request(
                    "GET",
                    "/api/coal-prices",
                    token,
                    Body::empty(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
            assert_eq!(body["ok"], false);
        }
    }

    #[tokio::test]
    async fn test_coal_price_history_checks_login_then_name_then_database() {
        for (token, uri, expected) in [
            (
                "forged",
                "/api/coal-prices/history?name=%E4%B8%B4%E5%8C%97",
                StatusCode::UNAUTHORIZED,
            ),
            (
                "test-session-token",
                "/api/coal-prices/history",
                StatusCode::BAD_REQUEST,
            ),
            (
                "test-session-token",
                "/api/coal-prices/history?name=",
                StatusCode::BAD_REQUEST,
            ),
            (
                "test-session-token",
                "/api/coal-prices/history?name=%E4%B8%B4%E5%8C%97",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            let response = test_app()
                .oneshot(session_request("GET", uri, token, Body::empty()))
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{uri}");
            let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
            assert_eq!(body["ok"], false);
        }
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
                "/api/master",
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
        let routes: [(&str, &str, &str); 8] = [
            ("GET", "/api/admin/settings", ""),
            ("PUT", "/api/admin/settings", r#"{"rank_interaction_k":50}"#),
            ("GET", "/api/admin/calibration", ""),
            ("GET", "/api/admin/csr-model", ""),
            ("GET", "/api/admin/keys", ""),
            ("DELETE", "/api/admin/keys/some-id", ""),
            (
                "POST",
                "/api/admin/benchmarks",
                r#"{"name":"b","props":{"S":1.3},"price":2000,"date":"2026-10-16"}"#,
            ),
            ("DELETE", "/api/admin/benchmarks/some-id", ""),
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

    // 密钥存在数据库里 (管理端生成/销毁), 所以"密钥有效"与"用途不符进不来"这两条路径需要真库,
    // 在库测试里 (`db_smoke_tests.rs` 的 `test_key_scopes_open_only_their_own_door`)。以下用例覆盖的是
    // **不需要库也必须成立**的部分: 缺密钥、会话冒充、畸形请求体不得绕过鉴权。

    /// MCP: 没带密钥 401 (碰库之前); 网页会话 Cookie 不算; GET 不开推送流, 回 405.
    #[tokio::test]
    async fn test_mcp_requires_key_and_rejects_get() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let response = test_app()
            .oneshot(key_request("POST", "/mcp", None, Body::from(body)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = test_app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(header::COOKIE, "doudou_session=test-session-token")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = test_app()
            .oneshot(Request::builder().uri("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    fn auth_from(pairs: &[(&str, &str)], on_railway: bool) -> Result<AuthConfig, String> {
        AuthConfig::from_lookup(on_railway, |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    /// 没显式配置 AUTH_* (用上了内置默认口令) 只监听本机回环; 三项都配了才对外监听.
    #[test]
    fn test_default_credentials_listen_on_loopback_only() {
        use std::net::{IpAddr, Ipv4Addr};
        let explicit = [
            ("AUTH_USERNAME", "someone"),
            ("AUTH_PASSWORD", "a-real-password"),
            (
                "AUTH_SESSION_TOKEN",
                "a-long-random-session-token-0123456789",
            ),
        ];
        let config = auth_from(&explicit, false).unwrap();
        assert!(!config.default_credentials);
        assert_eq!(config.listen_ip(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));

        let config = auth_from(&[], false).unwrap();
        assert!(config.default_credentials);
        assert_eq!(config.listen_ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));

        // 少配任何一项, 那一项就是内置默认值: 同样不对外.
        for missing in 0..explicit.len() {
            let mut partial = explicit.to_vec();
            partial.remove(missing);
            let config = auth_from(&partial, false).unwrap();
            assert!(config.default_credentials, "少了 {}", explicit[missing].0);
            assert_eq!(config.listen_ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        }

        // 代码里显式给的凭据 (测试、嵌入) 不算默认口令.
        let config = AuthConfig::new("tester", "secret", "test-session-token", false);
        assert_eq!(config.listen_ip(), IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    /// 变量在而值是空串 / 只有空白, 等于没设: 本机退回默认口令只听回环, Railway 上起不来.
    /// (曾经当成"显式配置了": 对外监听, 而且空的会话令牌和空 Cookie 对得上, 谁都算已登录.)
    #[test]
    fn test_blank_credentials_count_as_unset() {
        use std::net::{IpAddr, Ipv4Addr};
        let good = [
            ("AUTH_USERNAME", "someone"),
            ("AUTH_PASSWORD", "a-real-password"),
            (
                "AUTH_SESSION_TOKEN",
                "a-long-random-session-token-0123456789",
            ),
        ];
        for blank in ["", "   ", "\t"] {
            for index in 0..good.len() {
                let mut pairs = good.to_vec();
                pairs[index].1 = blank;
                let name = pairs[index].0;
                let config = auth_from(&pairs, false).unwrap();
                assert!(config.default_credentials, "{name}={blank:?}");
                assert_eq!(config.listen_ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
                assert!(!config.session_token.trim().is_empty());
                assert!(!config.username.trim().is_empty());
                assert!(!config.password.trim().is_empty());
                let error = auth_from(&pairs, true).err().unwrap();
                assert!(error.contains(name), "{name}={blank:?}: {error}");
            }
        }
        // 管理员三项里有空的: 管理端停用, 不拿空令牌当管理员令牌.
        let mut pairs = good.to_vec();
        pairs.extend([
            ("ADMIN_USERNAME", "boss"),
            ("ADMIN_PASSWORD", "admin-secret"),
            ("ADMIN_SESSION_TOKEN", ""),
        ]);
        assert!(auth_from(&pairs, true).unwrap().admin.is_none());
    }

    /// 空的会话 Cookie 永远不算登录 —— 哪怕代码里把令牌配成了空串.
    #[tokio::test]
    async fn test_empty_session_cookie_never_authenticates() {
        let configs = [
            AuthConfig::new("tester", "secret", "", false),
            AuthConfig::new("tester", "secret", "test-session-token", false),
        ];
        for auth in configs {
            let headers = |cookie: &str| {
                let mut headers = HeaderMap::new();
                headers.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
                headers
            };
            for cookie in [
                "doudou_session=",
                "doudou_session=; other=1",
                "doudou_session",
            ] {
                assert!(
                    session_role(&headers(cookie), &auth).is_none(),
                    "{cookie:?} 不该算登录"
                );
            }
            let response = app_with_auth(PathBuf::from("missing-public"), auth)
                .oneshot(session_request("GET", "/api/master", "", Body::empty()))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    /// Railway 上的行为不变: 三项必须都配, 缺了起不来; 配齐了对外监听、Cookie 带 Secure.
    #[test]
    fn test_railway_requires_explicit_credentials() {
        let error = auth_from(&[("AUTH_USERNAME", "someone")], true)
            .err()
            .unwrap();
        assert!(error.contains("AUTH_PASSWORD"), "{error}");
        let config = auth_from(
            &[
                ("AUTH_USERNAME", "someone"),
                ("AUTH_PASSWORD", "a-real-password"),
                (
                    "AUTH_SESSION_TOKEN",
                    "a-long-random-session-token-0123456789",
                ),
            ],
            true,
        )
        .unwrap();
        assert!(config.secure_cookie && !config.default_credentials);
        assert!(config.listen_ip().is_unspecified());
    }

    /// 把这个线程上的 span 与事件的字段全收下来 (不分级别), 给访问日志的测试看.
    #[derive(Clone, Default)]
    struct CapturedLog(Arc<std::sync::Mutex<Vec<String>>>);

    impl tracing::field::Visit for CapturedLog {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            let line = format!("{}={value:?}", field.name());
            self.0.lock().unwrap().push(line);
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturedLog {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _context: tracing_subscriber::layer::Context<'_, S>,
        ) {
            attrs.record(&mut self.clone());
        }

        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _context: tracing_subscriber::layer::Context<'_, S>,
        ) {
            event.record(&mut self.clone());
        }
    }

    /// 访问日志只记路由模板: `/mcp/{key}` 路径里的密钥, 日志开到最细也不进日志;
    /// 没匹配上路由的地址 (多写了一段) 同样不记.
    #[tokio::test]
    async fn test_access_log_never_contains_the_key_in_the_path() {
        use tracing_subscriber::layer::SubscriberExt;
        let captured = CapturedLog::default();
        let subscriber = tracing_subscriber::registry().with(captured.clone());
        let guard = tracing::subscriber::set_default(subscriber);
        let logged = || captured.0.lock().unwrap().join("\n");
        // 别的测试线程可能正好在首次登记访问日志的 span (那一刻还看不到这里的订阅者, 被记成
        // "没人要"), 所以重建登记后重发, 直到 span 确实被收下来 —— 否则下面 "不含密钥" 是空话.
        for _ in 0..200 {
            tracing::callsite::rebuild_interest_cache();
            for uri in ["/mcp/dk_secret-in-the-path", "/mcp/dk_secret-in-the-path/x"] {
                let request = key_request("POST", uri, None, Body::from("{}"));
                test_app().oneshot(request).await.unwrap();
            }
            if logged().contains("route=") {
                break;
            }
            tokio::task::yield_now().await;
        }
        drop(guard);
        let log = logged();
        assert!(
            log.contains("route=\"/mcp/{key}\""),
            "span 应带路由模板: {log}"
        );
        assert!(log.contains("route=\"-\""), "没匹配上的地址记 -: {log}");
        assert!(!log.contains("dk_secret"), "密钥进了日志: {log}");
    }

    /// 明确公开的路由. 往这里加一条要有理由: 不在这里的路由, 不带凭据一律 401.
    const PUBLIC_ROUTES: [(Verb, &str); 7] = [
        (Verb::Get, "/api/health"),
        (Verb::Get, "/api/coal-index"),
        (Verb::Get, "/api/auth/session"),
        (Verb::Post, "/api/auth/login"),
        (Verb::Post, "/api/auth/logout"),
        (Verb::Get, "/mcp"),
        (Verb::Get, "/mcp/{key}"),
    ];

    /// 路由表里的每一条, 不带任何凭据去请求 (写接口带一个畸形 JSON 体): 除了明确公开的,
    /// 一律 401 —— 不是 422 / 400 (先解析了请求体) 也不是 200 (忘了鉴权). 清单取自路由表本身,
    /// 新加的路由自动在内; 想公开一条新路由, 得同时改上面的清单.
    #[tokio::test]
    async fn test_every_route_without_credentials_gets_401() {
        let table: Vec<(Access, Verb, &str)> = routes()
            .iter()
            .map(|def| (def.access, def.verb, def.path))
            .collect();
        let public: Vec<(Verb, &str)> = table
            .iter()
            .filter(|(access, ..)| *access == Access::Public)
            .map(|(_, verb, path)| (*verb, *path))
            .collect();
        assert_eq!(public, PUBLIC_ROUTES, "公开路由清单变了");

        let guarded: Vec<_> = table
            .into_iter()
            .filter(|(access, ..)| *access != Access::Public)
            .collect();
        assert!(guarded.len() >= 30, "路由表不该只剩 {} 条", guarded.len());
        for (access, verb, path) in guarded {
            // 路径参数随便填一个值.
            let uri: String = path
                .split('/')
                .map(|segment| {
                    if segment.starts_with('{') {
                        "x"
                    } else {
                        segment
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            for app in [test_app(), app_without_admin()] {
                let body = match verb {
                    Verb::Get => Body::empty(),
                    Verb::Post | Verb::Put | Verb::Patch | Verb::Delete => Body::from("{not json"),
                };
                let request = Request::builder()
                    .method(verb.method())
                    .uri(&uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body)
                    .unwrap();
                // `/mcp/{key}` 的路径本身就是凭据: 填的是一把假密钥, 这里没配数据库验不了, 回 503
                // (同样在读请求体之前); 配了库时假密钥回 401, 见库测试.
                let expected = if path.contains("{key}") {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::UNAUTHORIZED
                };
                let response = app.oneshot(request).await.unwrap();
                assert_eq!(response.status(), expected, "{verb:?} {path} ({access:?})");
                let value: Value = serde_json::from_str(&response_text(response).await).unwrap();
                assert_eq!(value["ok"], false, "{verb:?} {path}");
            }
        }
    }

    /// 路由表里每一条按它登记的 Access, 分别用 "不带凭据 / 普通会话 / 管理员会话" 去敲门, 看放不放行.
    /// handler 换成一个只回 "到了" 的桩, 所以结果只取决于鉴权这一层, 不用数据库.
    /// (上面那条测试只管 "不带凭据 → 401", 管不到 "普通会话进了管理接口".)
    #[tokio::test]
    async fn test_each_access_class_admits_only_its_own_credentials() {
        const USER: &str = "test-session-token";
        const ADMIN: &str = "admin-session-token-0123456789abcdef";
        let state = AppState {
            auth: AuthConfig::new("tester", "secret", USER, false).with_admin(
                "boss",
                "admin-secret",
                ADMIN,
            ),
            database: None,
            rank_k: Arc::default(),
        };
        let ok = StatusCode::OK;
        let no = StatusCode::UNAUTHORIZED;
        for def in routes() {
            // 期望: [不带凭据, 普通会话, 管理员会话]. 穷尽 match: 新加一类 Access 必须写它的矩阵.
            let expected = match def.access {
                Access::Public => [ok, ok, ok],
                Access::Session => [no, ok, ok],
                Access::Admin => [no, StatusCode::FORBIDDEN, ok],
                // 普通会话不是钥匙: 没带 X-API-Key 就是 401.
                Access::DataWriter => [no, no, ok],
                // 会话 Cookie 进不了 MCP, 管理员的也不行 —— 只认 mcp 用途的密钥.
                Access::McpKey => [no, no, no],
            };
            // 路径本身带密钥的 (`/mcp/{key}`) 要查库验密钥, 这里没有库, 由库测试覆盖.
            if def.path.contains("{key}") {
                continue;
            }
            let uri: String = def
                .path
                .split('/')
                .map(|part| if part.starts_with('{') { "x" } else { part })
                .collect::<Vec<_>>()
                .join("/");
            let stub = on(def.verb.filter(), || async { "到了" });
            let router = Router::new()
                .route(def.path, def.access.guard(stub, &state))
                .with_state(state.clone());
            for (who, token, want) in [
                ("不带凭据", None, expected[0]),
                ("普通会话", Some(USER), expected[1]),
                ("管理员会话", Some(ADMIN), expected[2]),
            ] {
                let request = match token {
                    Some(token) => {
                        session_request(def.verb.method().as_str(), &uri, token, Body::empty())
                    }
                    None => key_request(def.verb.method().as_str(), &uri, None, Body::empty()),
                };
                let response = router.clone().oneshot(request).await.unwrap();
                assert_eq!(
                    response.status(),
                    want,
                    "{:?} {} ({:?}) · {who}",
                    def.verb,
                    def.path,
                    def.access
                );
            }
        }
    }

    /// 路径说了算的约定: 路由登记成哪一类不能和它的路径矛盾. 把管理接口登记成 "会话" 这类笔误,
    /// 上面两条测试都看不出来 (它们信路由表自己写的 Access), 这里按路径另算一遍.
    #[test]
    fn test_route_access_matches_its_path() {
        for def in routes() {
            let required = if def.path.starts_with("/api/admin/") {
                Some(Access::Admin)
            } else if def.path.starts_with("/api/master/") {
                // 煤库的写入与覆盖值清单 (`/api/master` 本身是只读的内置数据, 会话可读).
                Some(Access::DataWriter)
            } else if def.path.starts_with("/mcp") && def.verb != Verb::Get {
                Some(Access::McpKey)
            } else {
                None
            };
            if let Some(required) = required {
                assert_eq!(
                    def.access, required,
                    "{:?} {}: 这个路径下的路由必须登记成 {required:?}",
                    def.verb, def.path
                );
            }
            // 反过来: 会改数据的方法不许公开 (登录、登出除外).
            if def.access == Access::Public && def.verb != Verb::Get {
                assert!(
                    def.path.starts_with("/api/auth/"),
                    "{:?} {}: 写接口不能公开",
                    def.verb,
                    def.path
                );
            }
        }
    }

    /// 鉴权只管匹配上的 (路径, 方法): 路径在、方法不对照旧回 405 并列出可用的方法, 不变成 401.
    #[tokio::test]
    async fn test_wrong_method_on_a_guarded_path_is_still_405() {
        for (method, uri, allow) in [
            ("GET", "/api/solve", "POST"),
            ("DELETE", "/api/storage", "GET,HEAD,PUT"),
            ("PUT", "/mcp", "GET,HEAD,POST"),
        ] {
            let response = test_app()
                .oneshot(key_request(method, uri, None, Body::empty()))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {uri}"
            );
            let mut allowed: Vec<&str> = response.headers()[header::ALLOW]
                .to_str()
                .unwrap()
                .split(',')
                .map(str::trim)
                .collect();
            allowed.sort_unstable();
            assert_eq!(allowed.join(","), allow, "{method} {uri}");
        }
    }

    /// 路由只能经路由表挂上去: 绕过它直接挂的路由不在任何鉴权分组里, 上面的路由表测试也看不到.
    #[test]
    fn test_routes_are_only_mounted_through_the_table() {
        let production = production_source(include_str!("lib.rs"));
        assert_eq!(
            production.matches(".route(").count(),
            1,
            "lib.rs 里只有 api_router 可以调 .route(): 新路由加进 routes() 并写明 Access"
        );
        for bypass in [".route_service(", ".nest(", ".merge(", "::route(", "::nest"] {
            assert_eq!(
                production.matches(bypass).count(),
                0,
                "{bypass} 会绕过路由表: 新路由加进 routes() 并写明 Access"
            );
        }
        // 静态资源是唯一挂在路由表之外的服务, 而且不在 /api 与 /mcp 下.
        assert_eq!(
            production.matches(".nest_service(").count(),
            1,
            ".nest_service() 只给 /assets 用: 接口加进 routes() 并写明 Access"
        );
        assert!(production.contains(r#".nest_service("/assets", assets)"#));
    }

    /// `CLAUDE.md` 的规矩写成测试: MCP 的协议分派与排版、今日请求的组装是纯函数 —— 不查库、
    /// 不联网、不碰异步运行时与文件, 所以能全部用固定数据单测.
    #[test]
    fn test_pure_modules_do_no_io() {
        const PURE: [(&str, &str); 5] = [
            ("mcp.rs", include_str!("mcp.rs")),
            ("mcp_market.rs", include_str!("mcp_market.rs")),
            ("mcp_quote.rs", include_str!("mcp_quote.rs")),
            ("today.rs", include_str!("today.rs")),
            ("today_overrides.rs", include_str!("today_overrides.rs")),
        ];
        // "tokio" 不带 "::": `use tokio as rt;` 也得拦下; "std::{" 拦 `use std::{fs, …}` 这种合并写法.
        const FORBIDDEN: [&str; 12] = [
            "sqlx",
            "PgPool",
            "reqwest",
            "tokio",
            "block_on",
            "std::{",
            ".await",
            "async fn",
            "std::fs",
            "std::net",
            "std::env",
            "std::process",
        ];
        // 清单是手写的 (include_str! 只收字面量), 所以和代码对一遍账: 这些文件用 #[path] 拉进来的
        // 子模块必须也在清单里 —— 否则新拆出去的子模块不受这条测试管.
        for (file, source) in PURE {
            for line in production_source(source).lines() {
                let Some(child) = line
                    .trim()
                    .strip_prefix("#[path = \"")
                    .and_then(|rest| rest.strip_suffix("\"]"))
                else {
                    continue;
                };
                assert!(
                    PURE.iter().any(|(listed, _)| *listed == child),
                    "{file} 用 #[path] 拉进了 {child}: 把它加进这条测试的 PURE 清单"
                );
            }
        }
        for (file, source) in PURE {
            let production = production_source(source);
            assert!(production.len() > 1000, "{file} 的生产代码没读到");
            // 截掉的只能是文件末尾的测试模块: 后面再出现顶格的函数, 就是把生产代码也截掉了.
            let cut_off = &source[production.len()..];
            let top_level_fn = |line: &str| {
                let line = line
                    .trim_start_matches("pub ")
                    .trim_start_matches("pub(crate) ");
                let line = line.trim_start_matches("pub(super) ");
                line.starts_with("fn ") || line.starts_with("async fn ")
            };
            assert!(
                !cut_off.lines().any(top_level_fn),
                "{file}: 测试模块后面还有生产代码, 没查到"
            );
            for (at, line) in production.lines().enumerate() {
                // 注释里提到这些词不算.
                let code = line.split("//").next().unwrap_or(line);
                for word in FORBIDDEN {
                    assert!(
                        !code.contains(word),
                        "{file}:{} 出现了 `{word}`。规矩: mcp.rs 及其子模块、today.rs、\
                         today_overrides.rs 不查库不联网; 要用的线上数据加到 `mcp::Context`, \
                         在 lib.rs 的 `load_mcp_context` 里装好再传进来; 落库放在 lib.rs \
                         (如询价存档走 `quotes::store`)。",
                        at + 1
                    );
                }
            }
        }
        // 这条测试本身管用: 查库的模块按同样的办法查得出来.
        let impure = production_source(include_str!("quotes.rs"));
        assert!(impure.contains("sqlx") && impure.contains(".await"));
    }

    /// handler 要身份、路由却没挂在对应的鉴权组里 (没有中间件放身份): 当作没登录, 不放行.
    #[tokio::test]
    async fn test_identity_extractor_fails_closed_outside_its_group() {
        async fn needs_user(Authed(user): Authed<SessionUser>) -> String {
            user.username
        }
        let router = Router::new().route("/open", on(MethodFilter::GET, needs_user));
        let response = router
            .oneshot(session_request(
                "GET",
                "/open",
                "test-session-token",
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 未登录的写接口: 鉴权先于请求体解析, 一律 401, 不把字段名回显给访客.
    #[tokio::test]
    async fn test_unauthenticated_writes_get_401_before_body_parsing() {
        for (method, uri) in [
            ("PUT", "/api/storage"),
            ("POST", "/api/history"),
            ("POST", "/api/history/import"),
            ("PATCH", "/api/history/x/measured"),
            ("POST", "/api/admin/keys"),
        ] {
            let response = test_app()
                .oneshot(key_request(method, uri, None, Body::from("{}")))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
            let text = response_text(response).await;
            assert!(!text.contains("missing field"), "不得回显字段名: {text}");
        }
    }

    /// MCP 先验密钥再读请求体: 没带密钥时, 超过上限的请求体也回 401 而不是 413.
    #[tokio::test]
    async fn test_mcp_checks_key_before_reading_body() {
        for uri in ["/mcp", "/mcp/"] {
            let oversized = Body::from(vec![b' '; 3 * 1024 * 1024]);
            let response = test_app()
                .oneshot(key_request("POST", uri, None, oversized))
                .await
                .unwrap();
            let expected = if uri == "/mcp" {
                StatusCode::UNAUTHORIZED
            } else {
                // 路径里的密钥是空的: 对不上 `/mcp/{key}`, 落到静态资源兜底.
                StatusCode::METHOD_NOT_ALLOWED
            };
            assert_eq!(response.status(), expected, "{uri}");
        }
        let oversized = Body::from(vec![b' '; 3 * 1024 * 1024]);
        let response = test_app()
            .oneshot(key_request("POST", "/mcp/wrong-key", None, oversized))
            .await
            .unwrap();
        // 带了密钥但没配数据库: 验不了, 503; 同样发生在读请求体之前.
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

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

    /// 估价接口与基准列表: 要登录; 请求体坏了在估价前就 400 (不碰行情接口).
    #[tokio::test]
    async fn test_price_and_benchmark_endpoints_require_login() {
        for (method, uri) in [("POST", "/api/price"), ("GET", "/api/benchmarks")] {
            let anonymous = test_app()
                .oneshot(key_request(method, uri, None, Body::from("{}")))
                .await
                .unwrap();
            assert_eq!(
                anonymous.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }
        let bad = test_app()
            .oneshot(authenticated_request(
                "POST",
                "/api/price",
                Body::from("{not-json"),
            ))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    }

    /// 指数总览: 要登录; 没配数据库时照样列出 master 配置的序列, 只是没有值.
    #[tokio::test]
    async fn test_mcci_overview_requires_login_and_works_without_database() {
        let anonymous = test_app()
            .oneshot(key_request("GET", "/api/mcci", None, Body::empty()))
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

        let response = test_app()
            .oneshot(authenticated_request("GET", "/api/mcci", Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
        assert_eq!(body["ok"], true);
        for series in body["series"].as_array().unwrap() {
            assert!(series["code"].is_string());
            assert_eq!(series["points"], serde_json::json!([]));
        }
    }

    #[tokio::test]
    async fn test_admin_benchmark_rejects_bad_body_before_database() {
        let response = test_app()
            .oneshot(session_request(
                "POST",
                "/api/admin/benchmarks",
                "admin-session-token-0123456789abcdef",
                Body::from(r#"{"name":"b","props":{"S":1.3},"price":-1,"date":"2026-10-16"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let text = response_text(response).await;
        assert!(text.contains("价格无效"), "{text}");
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
    async fn test_master_endpoint() {
        let master_response = test_app()
            .oneshot(authenticated_request("GET", "/api/master", Body::empty()))
            .await
            .unwrap();
        let master: Value = serde_json::from_str(&response_text(master_response).await).unwrap();
        assert!(master.is_object());
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

    async fn static_response(uri: &str) -> Response {
        let public_dir = tempdir().unwrap();
        std::fs::write(public_dir.path().join("index.html"), "<html></html>").unwrap();
        std::fs::create_dir(public_dir.path().join("assets")).unwrap();
        std::fs::write(public_dir.path().join("assets/index-abc123.js"), "1").unwrap();
        app_with_auth(
            public_dir.path().to_path_buf(),
            AuthConfig::new("tester", "secret", "test-session-token", false),
        )
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
    }

    fn cache_control(response: &Response) -> Option<&str> {
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|value| value.to_str().unwrap())
    }

    #[tokio::test]
    async fn test_index_is_revalidated_after_every_deploy() {
        for uri in ["/", "/index.html", "/history"] {
            let response = static_response(uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(cache_control(&response), Some("no-cache"), "{uri}");
        }
    }

    #[tokio::test]
    async fn test_hashed_assets_cache_forever_but_missing_ones_do_not() {
        let response = static_response("/assets/index-abc123.js").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            cache_control(&response),
            Some("public, max-age=31536000, immutable")
        );

        // 上一版的资源已不在镜像里: 回 404, 不能把 index.html 当 JS 回, 也不能被长期缓存.
        let response = static_response("/assets/index-old999.js").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_ne!(
            cache_control(&response),
            Some("public, max-age=31536000, immutable")
        );
    }
}
