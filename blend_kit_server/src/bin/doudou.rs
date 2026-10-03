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
//! 档案存在 `$DOUDOU_CONFIG_DIR` (默认 `~/.config/doudou`) 的 profiles.json, 权限 600;
//! 只存会话 cookie, 不存密码. `--profile` 缺省取 `$DOUDOU_PROFILE`, 再缺省为 default.
//! 输出为接口返回的 JSON. 退出码: 0 = 成功; 1 = 接口报错或 ok=false; 2 = 用法错误.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_SERVER: &str = "https://doudou-blend.up.railway.app";
const SESSION_COOKIE: &str = "doudou_session";
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
        ["login", "--server", server, username] => Command::Login {
            server: Some((*server).into()),
            username: (*username).into(),
        },
        ["login", username] => Command::Login {
            server: None,
            username: (*username).into(),
        },
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
        _ => return Err(USAGE.into()),
    };
    Ok(Invocation { profile, command })
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

fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DOUDOU_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("doudou");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config").join("doudou")
}

fn load_store(dir: &Path) -> Result<Store, String> {
    match std::fs::read_to_string(dir.join("profiles.json")) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("档案文件损坏: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(error) => Err(format!("读取档案失败: {error}")),
    }
}

/// 写档案: 先写临时文件再改名, 权限 600 —— 里面是会话令牌.
fn save_store(dir: &Path, store: &Store) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("创建配置目录失败: {error}"))?;
    let path = dir.join("profiles.json");
    let temporary = dir.join("profiles.json.tmp");
    let text = serde_json::to_string_pretty(store).expect("Store 可序列化");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
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
    std::fs::rename(&temporary, &path).map_err(|error| format!("写档案失败: {error}"))
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

async fn run(invocation: Invocation, dir: &Path) -> Result<bool, String> {
    let client = reqwest::Client::new();
    let mut store = load_store(dir)?;
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
        save_store(dir.path(), &store).unwrap();
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
