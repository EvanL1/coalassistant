//! 焦煤现货价格指数代理 — 中价·新华焦煤价格指数 (CCP).
//!
//! 数据源: CCTD 中国煤炭市场网 `api.php?op=echarts`, 返回周频 JSON 序列
//! (长协/现货/竞价三条子指数, 2024 年至今). 由国家发改委价格监测中心 + 中国
//! 经济信息社(新华社)编制.
//!
//! 为什么要后端代理: CCTD 接口不回显 `Access-Control-Allow-Origin`, 浏览器直连
//! 会被 CORS 挡. 服务端 server-to-server 无此限制, 前端改调同源 `/api/coal-index`.
//!
//! 用途只是今日屏"若随现货指数变动"的参考估算, 不进求解 —— CCP 是全国基准指数,
//! 与用户实际中高硫山西煤仍有基差. 周频数据变化慢, 内存缓存 6 小时避免反复打源站; 过期后由一次后台外呼刷新,
//! 失败后十分钟内不再重试, 源站黑洞不拖慢求解.

use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

const SOURCE_URL: &str = "https://www.cctd.com.cn/api.php?op=echarts";
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 拉取失败后这么久内不再外呼, 直接用旧缓存 (或没有数据).
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(10 * 60);
/// 一条缓存都没有时, 请求最多陪外呼等这么久 (从外呼开始算); 源站黑洞时只拖慢这一小段.
const FIRST_FETCH_WAIT: Duration = Duration::from_secs(3);
/// 拉取任务超过这么久没回来 (比请求超时长) 就当它已经没了.
const FETCH_ABANDONED: Duration = Duration::from_secs(60);

/// 现货指数的一个周度点
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexPoint {
    /// 期末日 YYYY-MM-DD
    pub date: String,
    /// 现货指数点位
    pub price: f64,
}

/// CCTD echarts 原始记录. 只取现货指数(PRICE2), 其余字段忽略.
#[derive(Debug, Deserialize)]
struct RawRecord {
    #[serde(rename = "END_DATE")]
    end_date: String,
    #[serde(rename = "PRICE2")]
    price2: String,
}

/// 解析 CCTD 响应为升序的现货指数序列 (纯函数, 无 IO).
pub fn parse_ccp_spot(body: &str) -> Vec<IndexPoint> {
    let records: Vec<RawRecord> = match serde_json::from_str(body) {
        Ok(records) => records,
        Err(_) => return Vec::new(),
    };
    let mut points: Vec<IndexPoint> = records
        .into_iter()
        .filter_map(|record| {
            let price = record.price2.trim().parse::<f64>().ok()?;
            if !price.is_finite() || price <= 0.0 {
                return None;
            }
            Some(IndexPoint {
                date: record.end_date,
                price,
            })
        })
        .collect();
    points.sort_by(|a, b| a.date.cmp(&b.date));
    points
}

struct Cached {
    fetched_at: Instant,
    points: Vec<IndexPoint>,
}

#[derive(Default)]
struct CacheState {
    cached: Option<Cached>,
    /// 最近一次拉取失败的时间; 之后 [`SpotCache::retry_after_failure`] 内不再外呼.
    failed_at: Option<Instant>,
    /// 正在拉取 (后台任务) 的开始时间; 同一时刻只有一次外呼.
    fetching_since: Option<Instant>,
}

/// 现货指数缓存: 过期后只发一次外呼 (后台任务), 失败后一段时间内不再重试.
/// 有旧数据时调用方不等外呼, 直接拿旧的; 一条都没有时最多等 `first_wait` (从这次外呼开始算).
struct SpotCache {
    state: Mutex<CacheState>,
    done: Notify,
    ttl: Duration,
    retry_after_failure: Duration,
    first_wait: Duration,
}

type FetchResult = Result<Vec<IndexPoint>, String>;

impl SpotCache {
    fn new(ttl: Duration, retry_after_failure: Duration, first_wait: Duration) -> Self {
        Self {
            state: Mutex::new(CacheState::default()),
            done: Notify::new(),
            ttl,
            retry_after_failure,
            first_wait,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cached_any(&self) -> Option<Vec<IndexPoint>> {
        self.lock()
            .cached
            .as_ref()
            .map(|cached| cached.points.clone())
    }

    async fn get<F, Fut>(self: &Arc<Self>, fetch: F) -> Option<Vec<IndexPoint>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = FetchResult> + Send + 'static,
    {
        // 先登记再查状态: 查完到开始等之间完成的外呼也能叫醒这里.
        let done = self.done.notified();
        let deadline = {
            let mut state = self.lock();
            let now = Instant::now();
            if let Some(cached) = &state.cached {
                if now.duration_since(cached.fetched_at) < self.ttl {
                    return Some(cached.points.clone());
                }
            }
            let backing_off = state
                .failed_at
                .is_some_and(|failed| now.duration_since(failed) < self.retry_after_failure);
            if backing_off {
                return state.cached.as_ref().map(|cached| cached.points.clone());
            }
            // 拉取任务若异常消失, 超过 FETCH_ABANDONED 就当它没了, 重新发起.
            let since = match state.fetching_since {
                Some(since) if now.duration_since(since) < FETCH_ABANDONED => since,
                _ => {
                    state.fetching_since = Some(now);
                    let cache = Arc::clone(self);
                    let pending = fetch();
                    tokio::spawn(async move {
                        let result = pending.await;
                        cache.finish(result);
                    });
                    now
                }
            };
            if let Some(cached) = &state.cached {
                return Some(cached.points.clone());
            }
            since + self.first_wait
        };
        let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), done).await;
        self.cached_any()
    }

    fn finish(&self, result: FetchResult) {
        {
            let mut state = self.lock();
            let now = Instant::now();
            state.fetching_since = None;
            match result {
                Ok(points) if !points.is_empty() => {
                    state.cached = Some(Cached {
                        fetched_at: now,
                        points,
                    });
                    state.failed_at = None;
                }
                failed => {
                    let reason = failed.err().unwrap_or_else(|| "返回空序列".into());
                    tracing::warn!(%reason, "CCP 指数拉取失败, 暂用旧数据, 稍后再试");
                    state.failed_at = Some(now);
                }
            }
        }
        self.done.notify_waiters();
    }
}

static SPOT: LazyLock<Arc<SpotCache>> = LazyLock::new(|| {
    Arc::new(SpotCache::new(
        CACHE_TTL,
        RETRY_AFTER_FAILURE,
        FIRST_FETCH_WAIT,
    ))
});

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent("doudou-blend/1.0")
        .build()
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "CCP 客户端按配置创建失败, 用默认客户端");
            reqwest::Client::new()
        })
});

/// 取现货指数序列: 缓存新鲜直接返回; 过期则由一次后台外呼刷新, 有旧数据先给旧的;
/// 拉取失败后 [`RETRY_AFTER_FAILURE`] 内不再外呼. 返回 `None` 表示一条数据都没有.
pub async fn get_spot_series() -> Option<Vec<IndexPoint>> {
    SPOT.get(fetch_remote).await
}

async fn fetch_remote() -> FetchResult {
    let body = CLIENT
        .get(SOURCE_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    Ok(parse_ccp_spot(&body))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SAMPLE: &str = r#"[
        {"END_DATE":"2026-09-03","PRODUCT_NAME2":"现货指数","PRICE2":"1831"},
        {"END_DATE":"2026-06-25","PRODUCT_NAME2":"现货指数","PRICE2":"1503"},
        {"END_DATE":"2026-08-27","PRODUCT_NAME2":"现货指数","PRICE2":"1645"}
    ]"#;

    #[test]
    fn parses_spot_index_sorted_ascending() {
        let points = parse_ccp_spot(SAMPLE);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].date, "2026-06-25");
        assert_eq!(points[0].price, 1503.0);
        assert_eq!(points[2].date, "2026-09-03");
        assert_eq!(points[2].price, 1831.0);
    }

    #[test]
    fn skips_unparseable_or_nonpositive_prices() {
        let body = r#"[
            {"END_DATE":"2026-09-03","PRICE2":"1831"},
            {"END_DATE":"2026-09-04","PRICE2":"--"},
            {"END_DATE":"2026-09-05","PRICE2":"0"}
        ]"#;
        let points = parse_ccp_spot(body);
        assert_eq!(
            points,
            vec![IndexPoint {
                date: "2026-09-03".into(),
                price: 1831.0
            }]
        );
    }

    #[test]
    fn returns_empty_on_bad_json() {
        assert!(parse_ccp_spot("not json").is_empty());
        assert!(parse_ccp_spot("{}").is_empty());
        assert!(parse_ccp_spot("[]").is_empty());
    }

    fn sample_points() -> Vec<IndexPoint> {
        parse_ccp_spot(SAMPLE)
    }

    fn counting(
        calls: &Arc<AtomicUsize>,
        delay: Duration,
        result: FetchResult,
    ) -> impl FnOnce() -> std::pin::Pin<Box<dyn Future<Output = FetchResult> + Send>> {
        let calls = Arc::clone(calls);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                result
            })
        }
    }

    #[tokio::test]
    async fn test_concurrent_requests_share_one_fetch() {
        let cache = Arc::new(SpotCache::new(
            Duration::from_secs(3600),
            Duration::from_secs(3600),
            Duration::from_secs(5),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let fetch = counting(&calls, Duration::from_millis(50), Ok(sample_points()));
                tokio::spawn(async move { cache.get(fetch).await })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap(), Some(sample_points()));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // 新鲜缓存不再外呼.
        let fetch = counting(&calls, Duration::ZERO, Ok(vec![]));
        assert_eq!(cache.get(fetch).await, Some(sample_points()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_failure_is_remembered_for_a_while() {
        let cache = Arc::new(SpotCache::new(
            Duration::from_secs(3600),
            Duration::from_secs(3600),
            Duration::from_secs(5),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch = counting(&calls, Duration::ZERO, Err("源站挂了".into()));
        assert_eq!(cache.get(fetch).await, None);
        let started = Instant::now();
        for _ in 0..3 {
            let fetch = counting(&calls, Duration::ZERO, Ok(sample_points()));
            assert_eq!(cache.get(fetch).await, None);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_stale_cache_is_served_without_waiting_and_kept_on_failure() {
        // ttl 为 0: 每次都过期.
        let cache = Arc::new(SpotCache::new(
            Duration::ZERO,
            Duration::from_secs(3600),
            Duration::from_secs(5),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch = counting(&calls, Duration::ZERO, Ok(sample_points()));
        assert_eq!(cache.get(fetch).await, Some(sample_points()));

        let started = Instant::now();
        let slow_failure = counting(&calls, Duration::from_millis(200), Err("超时".into()));
        assert_eq!(cache.get(slow_failure).await, Some(sample_points()));
        assert!(started.elapsed() < Duration::from_millis(100));
        tokio::time::sleep(Duration::from_millis(300)).await;
        // 失败后旧数据仍在, 且退避期内不再外呼.
        let fetch = counting(&calls, Duration::ZERO, Ok(vec![]));
        assert_eq!(cache.get(fetch).await, Some(sample_points()));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_blackholed_source_delays_only_the_first_wait_window() {
        let cache = Arc::new(SpotCache::new(
            Duration::from_secs(3600),
            Duration::from_secs(3600),
            Duration::from_millis(100),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Instant::now();
        let fetch = counting(&calls, Duration::from_secs(3600), Ok(sample_points()));
        assert_eq!(cache.get(fetch).await, None);
        let first = started.elapsed();
        assert!(first >= Duration::from_millis(90) && first < Duration::from_millis(500));
        // 外呼还挂着: 后来的不再等, 也不再发第二次.
        let started = Instant::now();
        let fetch = counting(&calls, Duration::ZERO, Ok(sample_points()));
        assert_eq!(cache.get(fetch).await, None);
        assert!(started.elapsed() < Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
