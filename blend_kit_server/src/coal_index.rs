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
//! 与用户实际中高硫山西煤仍有基差. 周频数据变化慢, 内存缓存 6 小时避免反复打源站.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const SOURCE_URL: &str = "https://www.cctd.com.cn/api.php?op=echarts";
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

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

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

fn cached_fresh() -> Option<Vec<IndexPoint>> {
    let guard = CACHE.lock().ok()?;
    let cached = guard.as_ref()?;
    if cached.fetched_at.elapsed() < CACHE_TTL {
        Some(cached.points.clone())
    } else {
        None
    }
}

fn cached_any() -> Option<Vec<IndexPoint>> {
    let guard = CACHE.lock().ok()?;
    guard.as_ref().map(|cached| cached.points.clone())
}

fn store(points: Vec<IndexPoint>) {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(Cached {
            fetched_at: Instant::now(),
            points,
        });
    }
}

/// 取现货指数序列: 缓存新鲜直接返回; 过期则拉源站; 拉取失败退回旧缓存.
/// 返回 `None` 表示无缓存且本次抓取失败.
pub async fn get_spot_series() -> Option<Vec<IndexPoint>> {
    if let Some(points) = cached_fresh() {
        return Some(points);
    }
    match fetch_remote().await {
        Ok(points) if !points.is_empty() => {
            store(points.clone());
            Some(points)
        }
        // 抓取失败或解析空: 有旧缓存就退回旧的, 否则放弃
        _ => cached_any(),
    }
}

async fn fetch_remote() -> Result<Vec<IndexPoint>, reqwest::Error> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("doudou-blend/1.0")
        .build()?;
    let body = client.get(SOURCE_URL).send().await?.text().await?;
    Ok(parse_ccp_spot(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
