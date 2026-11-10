//! 求解与登录的防护: 求解并发上限与超时; 登录全局并发上限, 以及可选的按来源 IP 锁定.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::{
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 一次求解 (含排队) 的最长时间. 带价格稳健性分析的求解也远在此之内.
pub(crate) const SOLVE_TIMEOUT: Duration = Duration::from_secs(60);

/// 求解是纯 CPU 计算, 同时跑的不超过核数, 多出来的排队.
static SOLVER: LazyLock<Arc<Semaphore>> = LazyLock::new(|| {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    Arc::new(Semaphore::new(solver_permits(cores)))
});

/// 求解许可数: 核数, 至少 2. 容器常只报 1 核, 只给 1 个许可时一个慢求解会让其他人全排队.
fn solver_permits(cores: usize) -> usize {
    cores.max(2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComputeError {
    Timeout,
    Crashed,
}

impl ComputeError {
    /// 给用户看的原因; `what` 是这次算的东西 (求解 / 估价).
    pub(crate) fn reason(self, what: &str) -> String {
        match self {
            Self::Timeout => format!(
                "{what}超时 (超过 {} 秒): 服务器正忙或计算量过大, 请稍后重试",
                SOLVE_TIMEOUT.as_secs()
            ),
            Self::Crashed => format!("{what}线程异常退出"),
        }
    }
}

/// 在求解线程池里跑 `task`, 受全局并发上限与 [`SOLVE_TIMEOUT`] 约束.
pub(crate) async fn run_solver<T, F>(task: F) -> Result<T, ComputeError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    run_gated(Arc::clone(&SOLVER), SOLVE_TIMEOUT, task).await
}

/// 超时只是不再等结果: 阻塞线程停不下来, 许可随它一起留到算完才还, 免得超时的计算越堆越多.
async fn run_gated<T, F>(gate: Arc<Semaphore>, limit: Duration, task: F) -> Result<T, ComputeError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let work = async move {
        let permit = gate.acquire_owned().await.expect("求解信号量不会关闭");
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            task()
        })
        .await
    };
    match tokio::time::timeout(limit, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            tracing::error!(%error, "求解线程异常退出");
            Err(ComputeError::Crashed)
        }
        Err(_) => {
            tracing::warn!(seconds = limit.as_secs_f64(), "求解排队加计算超时");
            Err(ComputeError::Timeout)
        }
    }
}

/// 一段时间内允许的登录失败次数, 到了就锁这个来源.
const MAX_LOGIN_FAILURES: u32 = 5;
/// 失败计数的窗口: 距窗口开头超过这么久, 重新计数.
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// 第一次锁定的时长; 之后每次翻倍, 封顶 [`MAX_LOCKOUT`].
const FIRST_LOCKOUT: Duration = Duration::from_secs(60);
const MAX_LOCKOUT: Duration = Duration::from_secs(60 * 60);
/// 这么久没再失败的来源整条忘掉 (锁定时长也从头算).
const FORGET_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// 最多记多少个来源; 满了先清过期的, 仍满就丢最久没动的.
const MAX_TRACKED_SOURCES: usize = 10_000;
/// 同时在验密码的登录请求数. 失败的那 1 秒等待也占着名额, 全站猜密码的总速率因此有底.
const LOGIN_CONCURRENCY: usize = 4;
/// 排不上名额时最多等多久, 等不到回 429.
const LOGIN_QUEUE_WAIT: Duration = Duration::from_secs(10);
/// 打开按来源 IP 锁定的环境变量.
pub(crate) const IP_LOCKOUT_ENV: &str = "LOGIN_IP_LOCKOUT";
/// 日志里每个代理头最多记这么多字符.
const MAX_LOGGED_HEADER_CHARS: usize = 256;

/// `LOGIN_IP_LOCKOUT` 取 on / true / 1 (不分大小写) 才打开, 其余一律关闭.
pub(crate) fn ip_lockout_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "on" | "true" | "1"
        )
    })
}

#[derive(Debug, Clone)]
struct Failures {
    count: u32,
    window_start: Instant,
    lockouts: u32,
    locked_until: Option<Instant>,
    last_seen: Instant,
}

/// 登录防护: 全局并发上限 (总在) + 按来源计失败次数锁定 (默认关). 不按账号锁 —— 只有一个
/// 共用账号, 按账号锁等于让任何人都能把所有用户关在门外. 来源 IP 取自代理头, 代理若保留
/// 客户端自带的头, 按来源锁同样能被用来定向锁人, 所以要确认代理行为后显式打开.
pub(crate) struct LoginGuard {
    failures: Mutex<HashMap<String, Failures>>,
    capacity: usize,
    slots: Arc<Semaphore>,
    ip_lockout: bool,
}

impl Default for LoginGuard {
    fn default() -> Self {
        Self::with_ip_lockout(false)
    }
}

impl LoginGuard {
    pub(crate) fn with_ip_lockout(ip_lockout: bool) -> Self {
        Self::with_limits(MAX_TRACKED_SOURCES, LOGIN_CONCURRENCY, ip_lockout)
    }

    fn with_limits(capacity: usize, concurrency: usize, ip_lockout: bool) -> Self {
        Self {
            failures: Mutex::new(HashMap::new()),
            capacity,
            slots: Arc::new(Semaphore::new(concurrency)),
            ip_lockout,
        }
    }

    fn table(&self) -> std::sync::MutexGuard<'_, HashMap<String, Failures>> {
        self.failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 这个来源还要等多久才能再试; None = 现在就能试 (锁定关闭时总是 None).
    pub(crate) fn retry_after(&self, source: &str, now: Instant) -> Option<Duration> {
        if !self.ip_lockout {
            return None;
        }
        let table = self.table();
        let until = table.get(source)?.locked_until?;
        (until > now).then(|| until - now)
    }

    pub(crate) fn record_failure(&self, source: &str, now: Instant) {
        if !self.ip_lockout {
            return;
        }
        let mut table = self.table();
        if !table.contains_key(source) && table.len() >= self.capacity {
            table.retain(|_, entry| now.duration_since(entry.last_seen) < FORGET_AFTER);
            if table.len() >= self.capacity {
                let oldest = table
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_seen)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    table.remove(&oldest);
                }
            }
        }
        let fresh = Failures {
            count: 0,
            window_start: now,
            lockouts: 0,
            locked_until: None,
            last_seen: now,
        };
        let entry = table
            .entry(source.to_string())
            .or_insert_with(|| fresh.clone());
        if now.duration_since(entry.last_seen) >= FORGET_AFTER {
            *entry = fresh;
        }
        if now.duration_since(entry.window_start) >= FAILURE_WINDOW {
            entry.count = 0;
            entry.window_start = now;
        }
        entry.count += 1;
        entry.last_seen = now;
        if entry.count >= MAX_LOGIN_FAILURES {
            entry.lockouts += 1;
            let factor = 1u32 << (entry.lockouts - 1).min(16);
            entry.locked_until = Some(now + (FIRST_LOCKOUT * factor).min(MAX_LOCKOUT));
            entry.count = 0;
            entry.window_start = now;
        }
    }

    pub(crate) fn record_success(&self, source: &str) {
        self.table().remove(source);
    }

    /// 排一个验密码的名额; 等太久返回 None.
    pub(crate) async fn slot(&self) -> Option<OwnedSemaphorePermit> {
        tokio::time::timeout(LOGIN_QUEUE_WAIT, Arc::clone(&self.slots).acquire_owned())
            .await
            .ok()?
            .ok()
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.table().len()
    }
}

/// 登录请求的来源: 取 `X-Forwarded-For` 最左一项, 取不到再看 `X-Real-IP`, 再退回 TCP 对端
/// (本机直连). 最左一项只有在代理覆盖客户端自带的头时才可信, 见部署文档.
/// 只认能解析成 IP 的值, 键长因此有界; IPv6 按 /64 归桶 (一台主机通常独占一个 /64).
pub(crate) fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    let header_ip = |name: &str, pick_first: bool| {
        let value = headers.get(name)?.to_str().ok()?;
        let candidate = if pick_first {
            value.split(',').next()?
        } else {
            value
        };
        candidate.trim().parse::<IpAddr>().ok()
    };
    header_ip("x-forwarded-for", true)
        .or_else(|| header_ip("x-real-ip", false))
        .or_else(|| peer.map(|addr| addr.ip()))
        .map_or_else(|| "unknown".to_string(), bucket)
}

fn bucket(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V6(ip) => {
            let [a, b, c, d, ..] = ip.segments();
            format!("{}/64", Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
        }
        ip => ip.to_string(),
    }
}

/// 原样记下某个请求头的全部取值 (多条用 " | " 连接), 供日志判断代理是覆盖还是追加.
/// 没有记 "-"; 超长截断.
pub(crate) fn header_for_log(headers: &HeaderMap, name: &str) -> String {
    let values: Vec<String> = headers
        .get_all(name)
        .iter()
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        .collect();
    if values.is_empty() {
        return "-".into();
    }
    let joined = values.join(" | ");
    if joined.chars().count() <= MAX_LOGGED_HEADER_CHARS {
        joined
    } else {
        let mut cut: String = joined.chars().take(MAX_LOGGED_HEADER_CHARS).collect();
        cut.push('…');
        cut
    }
}

/// 429, 带 Retry-After (整秒, 向上取整, 至少 1).
pub(crate) fn too_many_logins(wait: Duration, reason: &str) -> Response {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    let seconds = seconds.max(1).to_string();
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, seconds)],
        Json(json!({ "authenticated": false, "reason": reason })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_gated_solver_caps_concurrency() {
        let gate = Arc::new(Semaphore::new(2));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..6)
            .map(|_| {
                let (gate, running, peak) = (gate.clone(), running.clone(), peak.clone());
                tokio::spawn(run_gated(gate, Duration::from_secs(10), move || {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                    running.fetch_sub(1, Ordering::SeqCst);
                }))
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap(), Ok(()));
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_gated_solver_times_out_and_keeps_permit_until_done() {
        let gate = Arc::new(Semaphore::new(1));
        let started = Instant::now();
        let slow = run_gated(gate.clone(), Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(400));
        })
        .await;
        assert_eq!(slow, Err(ComputeError::Timeout));
        assert!(started.elapsed() < Duration::from_millis(300));
        // 超时的计算还在跑, 名额没还: 后来的只能排队, 同样按时放弃.
        let queued = run_gated(gate.clone(), Duration::from_millis(50), || 1).await;
        assert_eq!(queued, Err(ComputeError::Timeout));
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(gate.available_permits(), 1);
        assert_eq!(run_gated(gate, Duration::from_secs(1), || 2).await, Ok(2));
    }

    #[tokio::test]
    async fn test_gated_solver_reports_panics() {
        let gate = Arc::new(Semaphore::new(1));
        let crashed = run_gated(gate.clone(), Duration::from_secs(5), || -> u8 {
            panic!("测试用的求解崩溃")
        })
        .await;
        assert_eq!(crashed, Err(ComputeError::Crashed));
        assert_eq!(gate.available_permits(), 1);
        assert!(ComputeError::Timeout.reason("求解").contains("超时"));
        assert!(ComputeError::Crashed.reason("求解").contains("异常"));
    }

    #[test]
    fn test_solver_permits_have_a_floor_of_two() {
        assert_eq!(solver_permits(1), 2);
        assert_eq!(solver_permits(2), 2);
        assert_eq!(solver_permits(8), 8);
    }

    #[test]
    fn test_ip_lockout_switch() {
        for value in ["on", "ON", "true", "1", " on "] {
            assert!(ip_lockout_enabled(Some(value)), "{value}");
        }
        for value in [
            None,
            Some(""),
            Some("off"),
            Some("0"),
            Some("false"),
            Some("yes?"),
        ] {
            assert!(!ip_lockout_enabled(value), "{value:?}");
        }
    }

    /// 锁定关闭 (默认): 错多少次都不锁, 也不记来源.
    #[test]
    fn test_login_guard_without_ip_lockout_never_locks() {
        let guard = LoginGuard::default();
        let now = Instant::now();
        for _ in 0..MAX_LOGIN_FAILURES * 4 {
            guard.record_failure("1.1.1.1", now);
        }
        assert_eq!(guard.retry_after("1.1.1.1", now), None);
        assert_eq!(guard.tracked(), 0);
    }

    #[test]
    fn test_login_lockout_grows_and_success_clears() {
        let guard = LoginGuard::with_ip_lockout(true);
        let now = Instant::now();
        for _ in 0..MAX_LOGIN_FAILURES - 1 {
            guard.record_failure("1.1.1.1", now);
        }
        assert_eq!(guard.retry_after("1.1.1.1", now), None);
        guard.record_failure("1.1.1.1", now);
        assert_eq!(guard.retry_after("1.1.1.1", now), Some(FIRST_LOCKOUT));
        // 别的来源不受影响.
        assert_eq!(guard.retry_after("2.2.2.2", now), None);

        // 锁过期后再错满一轮: 锁定翻倍.
        let later = now + FIRST_LOCKOUT;
        assert_eq!(guard.retry_after("1.1.1.1", later), None);
        for _ in 0..MAX_LOGIN_FAILURES {
            guard.record_failure("1.1.1.1", later);
        }
        assert_eq!(guard.retry_after("1.1.1.1", later), Some(FIRST_LOCKOUT * 2));

        // 封顶.
        let mut t = later;
        for _ in 0..20 {
            t += MAX_LOCKOUT;
            for _ in 0..MAX_LOGIN_FAILURES {
                guard.record_failure("1.1.1.1", t);
            }
        }
        assert_eq!(guard.retry_after("1.1.1.1", t), Some(MAX_LOCKOUT));

        guard.record_success("1.1.1.1");
        assert_eq!(guard.retry_after("1.1.1.1", t), None);
        for _ in 0..MAX_LOGIN_FAILURES {
            guard.record_failure("1.1.1.1", t);
        }
        assert_eq!(guard.retry_after("1.1.1.1", t), Some(FIRST_LOCKOUT));
    }

    #[test]
    fn test_login_failures_outside_window_do_not_add_up() {
        let guard = LoginGuard::with_ip_lockout(true);
        let mut t = Instant::now();
        for _ in 0..MAX_LOGIN_FAILURES * 3 {
            guard.record_failure("1.1.1.1", t);
            t += FAILURE_WINDOW / (MAX_LOGIN_FAILURES - 1) + Duration::from_secs(1);
            assert_eq!(guard.retry_after("1.1.1.1", t), None);
        }
    }

    #[test]
    fn test_login_guard_memory_is_bounded() {
        let guard = LoginGuard::with_limits(3, 1, true);
        let now = Instant::now();
        for i in 0..10u64 {
            guard.record_failure(&format!("10.0.0.{i}"), now + Duration::from_secs(i));
        }
        assert_eq!(guard.tracked(), 3);
        // 留下的是最近的几个.
        guard.record_failure("10.0.0.9", now + Duration::from_secs(20));
        assert_eq!(guard.tracked(), 3);
    }

    #[tokio::test]
    async fn test_login_slots_cap_concurrency() {
        let guard = LoginGuard::with_limits(10, 2, false);
        let a = guard.slot().await;
        let b = guard.slot().await;
        assert!(a.is_some() && b.is_some());
        assert_eq!(guard.slots.available_permits(), 0);
        drop(a);
        assert!(guard.slot().await.is_some());
    }

    #[test]
    fn test_client_ip_prefers_forwarded_for_then_real_ip_then_peer() {
        let mut headers = HeaderMap::new();
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(client_ip(&headers, Some(peer)), "127.0.0.1");
        assert_eq!(client_ip(&headers, None), "unknown");
        headers.insert("x-real-ip", "203.0.113.9".parse().unwrap());
        assert_eq!(client_ip(&headers, Some(peer)), "203.0.113.9");
        headers.insert(
            "x-forwarded-for",
            " 198.51.100.7 , 100.64.0.2".parse().unwrap(),
        );
        assert_eq!(client_ip(&headers, Some(peer)), "198.51.100.7");
        // 不是 IP 的值不当键.
        headers.insert("x-forwarded-for", "garbage".parse().unwrap());
        assert_eq!(client_ip(&headers, Some(peer)), "203.0.113.9");
        headers.insert("x-forwarded-for", "2001:db8::1".parse().unwrap());
        assert_eq!(client_ip(&headers, None), "2001:db8::/64");
    }

    #[test]
    fn test_client_ip_buckets_ipv6_by_slash_64() {
        let source = |ip: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", ip.parse().unwrap());
            client_ip(&headers, None)
        };
        assert_eq!(source("2001:db8:1:2::a"), "2001:db8:1:2::/64");
        assert_eq!(source("2001:db8:1:2:ffff:1:2:3"), "2001:db8:1:2::/64");
        assert_ne!(source("2001:db8:1:3::a"), source("2001:db8:1:2::a"));
        // IPv4 映射地址按 IPv4 算, 不并进 ::ffff:0:0/64 一个桶.
        assert_eq!(source("::ffff:198.51.100.7"), "198.51.100.7");
        let peer: SocketAddr = "[2001:db8:9:9::1]:443".parse().unwrap();
        assert_eq!(
            client_ip(&HeaderMap::new(), Some(peer)),
            "2001:db8:9:9::/64"
        );
    }

    #[test]
    fn test_header_for_log_keeps_every_value_and_caps_length() {
        let mut headers = HeaderMap::new();
        assert_eq!(header_for_log(&headers, "x-forwarded-for"), "-");
        headers.append("x-forwarded-for", "203.0.113.1".parse().unwrap());
        headers.append(
            "x-forwarded-for",
            "198.51.100.7, 100.64.0.2".parse().unwrap(),
        );
        assert_eq!(
            header_for_log(&headers, "x-forwarded-for"),
            "203.0.113.1 | 198.51.100.7, 100.64.0.2"
        );
        headers.insert("x-real-ip", "9".repeat(2000).parse().unwrap());
        let logged = header_for_log(&headers, "x-real-ip");
        assert!(
            logged.chars().count() <= MAX_LOGGED_HEADER_CHARS + 1,
            "{logged}"
        );
        assert!(logged.ends_with('…'));
    }

    #[test]
    fn test_too_many_logins_rounds_retry_after_up() {
        let response = too_many_logins(Duration::from_millis(1500), "稍后再试");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "2");
        let response = too_many_logins(Duration::ZERO, "稍后再试");
        assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    }
}
