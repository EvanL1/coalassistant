//! 豆哥配煤线上命令行: 登录线上服务, 用线上的数据和接口 (与网页同一套), 可切换多个账号.
//!
//! 与 `blend` 的区别: `blend` 在本机离线算, 只用内置 master; `doudou` 走线上 `/api/*`,
//! 带上煤库覆盖值、后台设置的 k、历史记录 —— 和用户在网页上看到的一致.
//!
//! ```text
//! doudou [--profile 名称] <命令>
//!   login [--server URL] <账号>    登录 (密码读 DOUDOU_PASSWORD, 否则提示输入)
//!   logout | whoami | profiles
//!   solve < request.json           求最低成本配方
//!   eval  < request.json           按 fixed_ratios 验算
//!   master                         线上煤库 (基线 + 覆盖值)
//!   history                        历史方案
//!   history measured <id> < x.json 回填实测化验 / 焦炭与炼焦条件
//!   admin settings [--k 数值|--off] 查看 / 设置煤阶交互 k (管理员)
//!   admin calibration|csr-model|keys|overrides
//!   api <METHOD> <路径> [< body]    任意接口
//! ```
//!
//! 档案存在 `$DOUDOU_CONFIG_DIR` (默认 `~/.config/doudou`) 的 profiles.json, 文件总是 600;
//! 默认目录与本程序新建的目录设为 700, 用户指定的已有目录不改权限, 对外开放时只提醒;
//! 只存会话 cookie, 不存密码. `--profile` 缺省取 `$DOUDOU_PROFILE`, 再缺省为 default.
//! 输出为接口返回的 JSON. 退出码: 0 = 成功; 1 = 接口报错或 ok=false; 2 = 用法错误.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_SERVER: &str = "https://doudou-blend.up.railway.app";
const SESSION_COOKIE: &str = "doudou_session";
/// 单次请求的总时长上限; 服务端求解自己最多 60 秒, 这里留足余量.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 认得的命令, 用来区分"命令不认识"和"参数不对".
const COMMANDS: [&str; 10] = [
    "login", "logout", "whoami", "profiles", "solve", "eval", "master", "history", "admin", "api",
];
const USAGE: &str =
    "用法: doudou [--profile 名称] login [--server URL] <账号> | logout | whoami | profiles \
| solve|eval < request.json | master | history [measured <id> < x.json] \
| admin settings [--k 数值|--off] | admin calibration|csr-model|keys|overrides \
| api <METHOD> <路径> [< body]";

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
struct Store {
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Profile {
    server: String,
    username: String,
    admin: bool,
    /// `doudou_session=<令牌>`, 原样放进 Cookie 头.
    cookie: String,
}

/// 一次接口调用: 方法、路径、请求体 (None = 无体; Some(Stdin) = 从标准输入读).
#[derive(Debug, PartialEq)]
enum Body {
    None,
    Stdin,
    Json(Value),
}

#[derive(Debug, PartialEq)]
enum Command {
    Login {
        server: Option<String>,
        username: String,
    },
    Logout,
    Whoami,
    Profiles,
    /// solve / eval: 要校验 fixed_ratios 与子命令是否一致.
    Solve {
        eval: bool,
    },
    Call {
        method: String,
        path: String,
        body: Body,
    },
}

#[derive(Debug, PartialEq)]
struct Invocation {
    profile: String,
    command: Command,
}

fn parse(args: &[String], default_profile: Option<String>) -> Result<Invocation, String> {
    let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut profile = default_profile.unwrap_or_else(|| "default".into());
    if let Some(index) = args.iter().position(|arg| *arg == "--profile") {
        let name = args.get(index + 1).ok_or("--profile 后面要跟名称")?;
        profile = (*name).to_string();
        args.drain(index..=index + 1);
    }
    let get = |path: &str| Command::Call {
        method: "GET".into(),
        path: path.into(),
        body: Body::None,
    };
    let command = match args.as_slice() {
        ["login", rest @ ..] => parse_login(rest)?,
        ["logout"] => Command::Logout,
        ["whoami"] => Command::Whoami,
        ["profiles"] => Command::Profiles,
        ["solve"] => Command::Solve { eval: false },
        ["eval"] => Command::Solve { eval: true },
        ["master"] => get("/api/master"),
        ["history"] => get("/api/history"),
        ["history", "measured", id] => Command::Call {
            method: "PATCH".into(),
            path: format!("/api/history/{}/measured", encode(id)),
            body: Body::Stdin,
        },
        ["admin", "settings"] => get("/api/admin/settings"),
        ["admin", "settings", "--off"] => Command::Call {
            method: "PUT".into(),
            path: "/api/admin/settings".into(),
            body: Body::Json(json!({ "rank_interaction_k": null })),
        },
        ["admin", "settings", "--k", value] => {
            let k: f64 = value.parse().map_err(|_| format!("k 不是数字: {value}"))?;
            Command::Call {
                method: "PUT".into(),
                path: "/api/admin/settings".into(),
                body: Body::Json(json!({ "rank_interaction_k": k })),
            }
        }
        ["admin", "calibration"] => get("/api/admin/calibration"),
        ["admin", "csr-model"] => get("/api/admin/csr-model"),
        ["admin", "keys"] => get("/api/admin/keys"),
        ["admin", "overrides"] => get("/api/master/overrides"),
        ["api", method, path] => {
            let method = method.to_ascii_uppercase();
            if !path.starts_with("/api/") {
                return Err("路径要以 /api/ 开头".into());
            }
            let body = match method.as_str() {
                "GET" | "DELETE" => Body::None,
                "POST" | "PUT" | "PATCH" => Body::Stdin,
                _ => return Err(format!("不支持的方法: {method}")),
            };
            Command::Call {
                method,
                path: (*path).into(),
                body,
            }
        }
        _ => return Err(usage_error(&args)),
    };
    Ok(Invocation { profile, command })
}

/// login 的参数: `--server URL` (或 `--server=URL`) 放在账号前后都行, 账号恰好一个.
fn parse_login(rest: &[&str]) -> Result<Command, String> {
    let mut server: Option<String> = None;
    let mut username: Option<String> = None;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        let value = match *arg {
            "--server" => Some(
                args.next()
                    .ok_or("login: --server 后面要跟服务地址, 如 --server https://example.com")?
                    .to_string(),
            ),
            other => other.strip_prefix("--server=").map(str::to_string),
        };
        if let Some(value) = value {
            if value.is_empty() || value.starts_with("--") {
                return Err(format!(
                    "login: --server 后面要跟服务地址, 收到的是 {value:?}"
                ));
            }
            if server.replace(value).is_some() {
                return Err("login: --server 只能给一次".into());
            }
        } else if arg.starts_with("--") {
            return Err(format!("login: 不认识的选项 {arg} (只有 --server)"));
        } else if let Some(first) = username.replace(arg.to_string()) {
            return Err(format!("login: 只能登录一个账号, 收到了 {first} 和 {arg}"));
        }
    }
    let username = username.ok_or("login: 缺少账号, 用法: doudou login [--server URL] <账号>")?;
    Ok(Command::Login { server, username })
}

/// 用法错误说清楚错在哪: 命令不认识, 还是认识但参数不对.
fn usage_error(args: &[&str]) -> String {
    match args {
        [] => USAGE.into(),
        [command, rest @ ..] if COMMANDS.contains(command) => {
            if rest.is_empty() {
                format!("{command}: 缺少参数\n{USAGE}")
            } else {
                format!("{command}: 参数不对: {}\n{USAGE}", rest.join(" "))
            }
        }
        [command, ..] => format!("不认识的命令: {command}\n{USAGE}"),
    }
}

/// 路径段编码: 历史 id 是 UUID, 这里只防意外字符, 不做完整 URL 编码.
fn encode(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// 从登录响应的 Set-Cookie 里取出会话 cookie (`doudou_session=<令牌>`).
fn session_cookie<'a>(set_cookies: impl IntoIterator<Item = &'a str>) -> Option<String> {
    set_cookies.into_iter().find_map(|header| {
        let pair = header.split(';').next()?.trim();
        let (name, value) = pair.split_once('=')?;
        (name == SESSION_COOKIE && !value.is_empty()).then(|| pair.to_string())
    })
}

/// 请求体与子命令是否一致: eval 必须带 fixed_ratios, solve 不能带.
fn check_solve_body(body: &str, eval: bool) -> Result<(), String> {
    let request: Value =
        serde_json::from_str(body).map_err(|error| format!("请求不是合法 JSON: {error}"))?;
    let has_fixed = request
        .get("fixed_ratios")
        .is_some_and(|value| !value.is_null());
    match (eval, has_fixed) {
        (true, false) => Err("eval 需要请求里带 fixed_ratios (煤名 → 份数)".into()),
        (false, true) => Err("solve 求最优, 请求里不应带 fixed_ratios; 要验算请用 eval".into()),
        _ => Ok(()),
    }
}

/// 接口返回是否算成功: HTTP 2xx 且 JSON 里没有 ok=false.
fn succeeded(status: u16, body: &str) -> bool {
    (200..300).contains(&status)
        && serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| value.get("ok").and_then(Value::as_bool))
            != Some(false)
}

/// 档案目录. `dedicated`: 本程序专用的默认目录 (…/doudou), 可以收紧权限;
/// 否则是用户用 `DOUDOU_CONFIG_DIR` 指定的, 可能是家目录或 /tmp 这类共用目录.
#[derive(Debug, Clone, PartialEq)]
struct ConfigDir {
    path: PathBuf,
    dedicated: bool,
}

impl ConfigDir {
    fn dedicated(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            dedicated: true,
        }
    }

    fn chosen(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            dedicated: false,
        }
    }
}

fn config_dir() -> ConfigDir {
    if let Some(dir) = std::env::var_os("DOUDOU_CONFIG_DIR") {
        return ConfigDir::chosen(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return ConfigDir::dedicated(PathBuf::from(dir).join("doudou"));
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    ConfigDir::dedicated(PathBuf::from(home).join(".config").join("doudou"))
}

fn load_store(dir: &Path) -> Result<Store, String> {
    match std::fs::read_to_string(dir.join("profiles.json")) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("档案文件损坏: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(error) => Err(format!("读取档案失败: {error}")),
    }
}

/// 写档案: 先写临时文件再改名. 里面是会话令牌, 文件总是 600. 目录: 本程序新建的、默认的专用
/// 目录收紧到 700 (旧版本可能留下宽权限); 用户指定的已有目录不动, 对外开放时只提醒.
fn save_store(dir: &ConfigDir, store: &Store) -> Result<(), String> {
    let root = dir.path.as_path();
    if !root.exists() {
        create_private_dir(root).map_err(|error| format!("创建配置目录失败: {error}"))?;
    } else if dir.dedicated {
        restrict(root, 0o700).map_err(|error| format!("设置配置目录权限失败: {error}"))?;
    } else if let Some(warning) = exposure_warning(root) {
        eprintln!("{warning}");
    }
    let path = root.join("profiles.json");
    let temporary = root.join("profiles.json.tmp");
    let text = serde_json::to_string_pretty(store).expect("Store 可序列化");
    // 上次中断可能留下旧的临时文件 (权限、链接都不可信): 删掉后新建, 不沿用.
    match std::fs::remove_file(&temporary) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("清理旧的临时档案失败: {error}"));
        }
        _ => {}
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("写档案失败: {error}"))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("写档案失败: {error}"))?;
    std::fs::rename(&temporary, &path).map_err(|error| format!("写档案失败: {error}"))?;
    restrict(&path, 0o600).map_err(|error| format!("设置档案权限失败: {error}"))
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    restrict(path, 0o700)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// 目录对组或其他用户开放时的提醒; 读不到权限或已收紧时为 None.
#[cfg(unix)]
fn exposure_warning(dir: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dir).ok()?.permissions().mode() & 0o777;
    (mode & 0o077 != 0).then(|| {
        format!(
            "警告: 配置目录 {} 对组或其他用户开放 (权限 {mode:o}); 档案文件本身为 600, \
             但建议把 DOUDOU_CONFIG_DIR 指向只有自己能进的目录",
            dir.display()
        )
    })
}

#[cfg(not(unix))]
fn exposure_warning(_dir: &Path) -> Option<String> {
    None
}

fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(CONNECT_TIMEOUT.min(timeout))
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))
}

fn read_stdin() -> Result<String, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("读取标准输入失败: {error}"))?;
    Ok(input)
}

/// 密码: 优先 DOUDOU_PASSWORD (给脚本 / agent), 否则在终端提示输入 (关回显).
fn read_password() -> Result<String, String> {
    if let Ok(password) = std::env::var("DOUDOU_PASSWORD") {
        return Ok(password);
    }
    let stdin = std::io::stdin();
    let interactive = stdin.is_terminal();
    if interactive {
        eprint!("密码: ");
        let _ = std::process::Command::new("stty").arg("-echo").status();
    }
    let mut line = String::new();
    let result = stdin.read_line(&mut line);
    if interactive {
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
    }
    result.map_err(|error| format!("读取密码失败: {error}"))?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

struct Outcome {
    status: u16,
    body: String,
}

async fn call(
    client: &reqwest::Client,
    profile: &Profile,
    method: &str,
    path: &str,
    body: Option<String>,
) -> Result<Outcome, String> {
    let method =
        reqwest::Method::from_bytes(method.as_bytes()).map_err(|error| error.to_string())?;
    let url = format!("{}{path}", profile.server.trim_end_matches('/'));
    let mut request = client
        .request(method, &url)
        .header(reqwest::header::COOKIE, &profile.cookie);
    if let Some(body) = body {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("连接 {url} 失败: {error}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|error| error.to_string())?;
    Ok(Outcome { status, body })
}

async fn login(
    client: &reqwest::Client,
    server: &str,
    username: &str,
    password: &str,
) -> Result<Profile, String> {
    let url = format!("{}/api/auth/login", server.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(json!({ "username": username, "password": password }).to_string())
        .send()
        .await
        .map_err(|error| format!("连接 {url} 失败: {error}"))?;
    let cookie = session_cookie(
        response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok()),
    );
    let status = response.status();
    let body: Value = response
        .text()
        .await
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    match cookie {
        Some(cookie) if status.is_success() => Ok(Profile {
            server: server.trim_end_matches('/').into(),
            username: username.into(),
            admin: body.get("admin").and_then(Value::as_bool) == Some(true),
            cookie,
        }),
        _ => Err(body
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("登录失败")
            .to_string()),
    }
}

async fn run(invocation: Invocation, dir: &ConfigDir) -> Result<bool, String> {
    let client = http_client(REQUEST_TIMEOUT)?;
    let mut store = load_store(&dir.path)?;
    let name = invocation.profile;
    let current = || {
        store.profiles.get(&name).cloned().ok_or_else(|| {
            format!("档案 {name} 未登录, 先运行: doudou --profile {name} login <账号>")
        })
    };
    let (method, path, body) = match invocation.command {
        Command::Login { server, username } => {
            let server = server
                .or_else(|| std::env::var("DOUDOU_SERVER").ok())
                .or_else(|| store.profiles.get(&name).map(|p| p.server.clone()))
                .unwrap_or_else(|| DEFAULT_SERVER.into());
            let password = read_password()?;
            let profile = login(&client, &server, &username, &password).await?;
            let role = if profile.admin {
                "管理员"
            } else {
                "普通用户"
            };
            eprintln!("已登录 {} ({role}) → 档案 {name}", profile.username);
            store.profiles.insert(name, profile);
            save_store(dir, &store)?;
            return Ok(true);
        }
        Command::Logout => {
            if let Some(profile) = store.profiles.remove(&name) {
                // 服务端会话令牌是固定的, 登出只是删掉本地档案; 请求失败不影响.
                let _ = call(&client, &profile, "POST", "/api/auth/logout", None).await;
                save_store(dir, &store)?;
            }
            eprintln!("档案 {name} 已登出");
            return Ok(true);
        }
        Command::Profiles => {
            let listing: BTreeMap<&String, Value> = store
                .profiles
                .iter()
                .map(|(key, p)| {
                    (
                        key,
                        json!({ "server": p.server, "username": p.username, "admin": p.admin }),
                    )
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&listing).expect("可序列化")
            );
            return Ok(true);
        }
        Command::Whoami => {
            let profile = current()?;
            let outcome = call(&client, &profile, "GET", "/api/auth/session", None).await?;
            let session: Value = serde_json::from_str(&outcome.body).unwrap_or(Value::Null);
            let authenticated = session.get("authenticated").and_then(Value::as_bool) == Some(true);
            println!(
                "{}",
                json!({
                    "profile": name,
                    "server": profile.server,
                    "username": profile.username,
                    "authenticated": authenticated,
                    "admin": session.get("admin").and_then(Value::as_bool) == Some(true),
                })
            );
            return Ok(authenticated);
        }
        Command::Solve { eval } => {
            let input = read_stdin()?;
            // 与 blend 一致: 请求体与子命令不符是用法错误, 退出码 2.
            if let Err(message) = check_solve_body(&input, eval) {
                eprintln!("{message}");
                std::process::exit(2);
            }
            ("POST".to_string(), "/api/solve".to_string(), Some(input))
        }
        Command::Call { method, path, body } => {
            let body = match body {
                Body::None => None,
                Body::Stdin => Some(read_stdin()?),
                Body::Json(value) => Some(value.to_string()),
            };
            (method, path, body)
        }
    };
    let profile = current()?;
    let outcome = call(&client, &profile, &method, &path, body).await?;
    println!("{}", outcome.body);
    if outcome.status == 401 {
        eprintln!(
            "会话已失效, 重新登录: doudou --profile {name} login {}",
            profile.username
        );
    }
    Ok(succeeded(outcome.status, &outcome.body))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let invocation = match parse(&args, std::env::var("DOUDOU_PROFILE").ok()) {
        Ok(invocation) => invocation,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(invocation, &config_dir()).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn test_profile_flag_anywhere_and_env_default() {
        let parsed = parse(&args("whoami --profile admin"), None).unwrap();
        assert_eq!(parsed.profile, "admin");
        let parsed = parse(&args("whoami"), Some("ops".into())).unwrap();
        assert_eq!(parsed.profile, "ops");
        assert_eq!(parse(&args("whoami"), None).unwrap().profile, "default");
        assert!(parse(&args("whoami --profile"), None).is_err());
    }

    #[test]
    fn test_admin_settings_commands() {
        let parsed = parse(&args("admin settings --k 66.2"), None).unwrap();
        assert_eq!(
            parsed.command,
            Command::Call {
                method: "PUT".into(),
                path: "/api/admin/settings".into(),
                body: Body::Json(json!({ "rank_interaction_k": 66.2 })),
            }
        );
        let off = parse(&args("admin settings --off"), None).unwrap();
        assert!(
            matches!(off.command, Command::Call { body: Body::Json(ref v), .. } if v["rank_interaction_k"].is_null())
        );
        assert!(parse(&args("admin settings --k abc"), None).is_err());
    }

    #[test]
    fn test_generic_api_and_usage_errors() {
        let parsed = parse(&args("api post /api/solve"), None).unwrap();
        assert_eq!(
            parsed.command,
            Command::Call {
                method: "POST".into(),
                path: "/api/solve".into(),
                body: Body::Stdin
            }
        );
        assert!(parse(&args("api GET /etc/passwd"), None).is_err());
        assert!(parse(&args("api TRACE /api/x"), None).is_err());
        assert!(parse(&args("frobnicate"), None).is_err());
        assert!(parse(&[], None).is_err());
    }

    #[test]
    fn test_login_accepts_server_flag_anywhere() {
        let expected = Command::Login {
            server: Some("http://x".into()),
            username: "zhangsan".into(),
        };
        for text in [
            "login zhangsan --server http://x",
            "login --server http://x zhangsan",
            "login --server=http://x zhangsan",
        ] {
            assert_eq!(
                parse(&args(text), None).unwrap().command,
                expected,
                "{text}"
            );
        }
        assert_eq!(
            parse(&args("login zhangsan"), None).unwrap().command,
            Command::Login {
                server: None,
                username: "zhangsan".into()
            }
        );
    }

    #[test]
    fn test_usage_errors_say_what_is_wrong() {
        let error = |text: &str| parse(&args(text), None).unwrap_err();
        assert!(error("login").contains("缺少账号"));
        assert!(error("login zhangsan --server").contains("--server"));
        assert!(error("login a b").contains('b'));
        assert!(error("login a --port 1").contains("--port"));
        assert!(error("login --server x --server y a").contains("一次"));
        assert!(error("frobnicate").contains("不认识的命令: frobnicate"));
        assert!(error("history measured").contains("history: 参数不对: measured"));
        assert!(error("api").contains("api: 缺少参数"));
    }

    #[tokio::test]
    async fn test_http_client_gives_up_on_a_silent_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        // 收下连接但从不回应.
        let _hold = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let profile = Profile {
            server: format!("http://{address}"),
            username: "u".into(),
            admin: false,
            cookie: "doudou_session=t".into(),
        };
        let client = http_client(Duration::from_millis(200)).unwrap();
        let started = std::time::Instant::now();
        let result = call(&client, &profile, "GET", "/api/auth/session", None).await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn test_history_measured_encodes_id() {
        let parsed = parse(&args("history measured a/b"), None).unwrap();
        assert!(
            matches!(parsed.command, Command::Call { ref path, .. } if path == "/api/history/a%2Fb/measured")
        );
    }

    #[test]
    fn test_session_cookie_extraction() {
        let headers = ["other=1; Path=/", "doudou_session=abc123; Path=/; HttpOnly"];
        assert_eq!(
            session_cookie(headers),
            Some("doudou_session=abc123".into())
        );
        assert_eq!(session_cookie(["doudou_session=; Max-Age=0"]), None);
        assert_eq!(session_cookie([]), None);
    }

    #[test]
    fn test_solve_body_must_match_subcommand() {
        assert!(check_solve_body(r#"{"coals":[]}"#, false).is_ok());
        assert!(check_solve_body(r#"{"fixed_ratios":{"a":1}}"#, true).is_ok());
        assert!(check_solve_body(r#"{"fixed_ratios":{"a":1}}"#, false).is_err());
        assert!(check_solve_body(r#"{"coals":[]}"#, true).is_err());
        assert!(check_solve_body("not json", false).is_err());
    }

    #[test]
    fn test_success_rule() {
        assert!(succeeded(200, r#"{"ok":true}"#));
        assert!(succeeded(200, r#"{"authenticated":true}"#));
        assert!(!succeeded(200, r#"{"ok":false,"reason":"x"}"#));
        assert!(!succeeded(403, r#"{"ok":false}"#));
    }

    #[test]
    fn test_store_roundtrip_is_private() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_store(dir.path()).unwrap(), Store::default());
        let mut store = Store::default();
        store.profiles.insert(
            "admin".into(),
            Profile {
                server: "http://x".into(),
                username: "u".into(),
                admin: true,
                cookie: "doudou_session=t".into(),
            },
        );
        save_store(&ConfigDir::chosen(dir.path()), &store).unwrap();
        assert_eq!(load_store(dir.path()).unwrap(), store);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("profiles.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// 默认的专用目录: 旧版本留下的宽权限目录与档案, 写一次就收紧.
    #[cfg(unix)]
    #[test]
    fn test_store_tightens_the_dedicated_dir() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("doudou");
        std::fs::create_dir(&dir).unwrap();
        set_mode(&dir, 0o755);
        std::fs::write(dir.join("profiles.json"), "{}").unwrap();
        set_mode(&dir.join("profiles.json"), 0o644);
        save_store(&ConfigDir::dedicated(&dir), &Store::default()).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("profiles.json")), 0o600);
    }

    /// 用户用 DOUDOU_CONFIG_DIR 指定的已有目录 (家目录、/tmp 之类) 不改权限, 只提醒.
    #[cfg(unix)]
    #[test]
    fn test_store_leaves_a_chosen_existing_dir_alone() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("shared");
        std::fs::create_dir(&dir).unwrap();
        set_mode(&dir, 0o755);
        save_store(&ConfigDir::chosen(&dir), &Store::default()).unwrap();
        assert_eq!(mode(&dir), 0o755);
        assert_eq!(mode(&dir.join("profiles.json")), 0o600);
        assert!(exposure_warning(&dir).unwrap().contains("755"));
        set_mode(&dir, 0o700);
        assert_eq!(exposure_warning(&dir), None);
    }

    /// 指定的目录还不存在: 由本程序新建, 建成 700.
    #[cfg(unix)]
    #[test]
    fn test_store_creates_a_missing_chosen_dir_private() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a").join("doudou");
        save_store(&ConfigDir::chosen(&dir), &Store::default()).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("profiles.json")), 0o600);
    }

    /// 上次中断留下的临时文件先删掉再新建, 不往旧文件 (可能被别人硬链接着) 里写令牌.
    #[cfg(unix)]
    #[test]
    fn test_store_replaces_a_stale_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("doudou");
        std::fs::create_dir(&dir).unwrap();
        let stale = dir.join("profiles.json.tmp");
        std::fs::write(&stale, "stale").unwrap();
        set_mode(&stale, 0o644);
        let other = root.path().join("other");
        std::fs::hard_link(&stale, &other).unwrap();
        let mut store = Store::default();
        store.profiles.insert(
            "default".into(),
            Profile {
                server: "http://x".into(),
                username: "u".into(),
                admin: false,
                cookie: "doudou_session=secret".into(),
            },
        );
        save_store(&ConfigDir::dedicated(&dir), &store).unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "stale");
        assert!(!stale.exists());
        assert_eq!(load_store(&dir).unwrap(), store);
        assert_eq!(mode(&dir.join("profiles.json")), 0o600);
    }

    /// 端到端: 起一个真实的本地服务, 两个档案分别以普通用户和管理员登录.
    #[tokio::test]
    async fn test_two_profiles_against_live_server() {
        let auth = blend_kit_server::AuthConfig::new(
            "user",
            "pw",
            "user-session-token-0123456789abcdef",
            false,
        )
        .with_admin("boss", "adminpw", "admin-session-token-0123456789abcdef");
        let public = tempfile::tempdir().unwrap();
        let app = blend_kit_server::app_with_auth(public.path().to_path_buf(), auth);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = reqwest::Client::new();
        let user = login(&client, &server, "user", "pw").await.unwrap();
        let admin = login(&client, &server, "boss", "adminpw").await.unwrap();
        assert!(!user.admin && admin.admin);
        assert!(login(&client, &server, "user", "wrong").await.is_err());

        let as_user = call(&client, &user, "GET", "/api/admin/settings", None)
            .await
            .unwrap();
        assert_eq!(as_user.status, 403);
        let master = call(&client, &admin, "GET", "/api/master", None)
            .await
            .unwrap();
        assert_eq!(master.status, 200);
        let request = r#"{"coals":[{"name":"a","fob":1000,"frt":0,"props":{"S":0.5,"A":9,"V":24,"G":85,"Y":16,"petro":0.1,"CSR":66,"M":8}}],"specs":[]}"#;
        let solved = call(&client, &user, "POST", "/api/solve", Some(request.into()))
            .await
            .unwrap();
        assert!(succeeded(solved.status, &solved.body), "{}", solved.body);
    }
}
