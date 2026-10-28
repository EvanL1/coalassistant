//! 钢铁网 (Mysteel) 国产炼焦煤价格指数 (MCCI): 每天拉一次存库, 给估价当分煤种的基准报价.
//!
//! 为什么用它: 按方法论 (《煤焦价格指数方法论》2026-05) 每个分指数是一个固定规格的标准品,
//! 由二十多家山西矿/洗煤厂的成交、询盘、报盘按指标线性标准化后按量加权, 每个交易日一个值
//! (元/湿吨, 现金). 规格固定 + 日更, 正好当"每天对标"的基准; 标准品指标写在 master
//! `price_factor.index_series`, 这里只管取数.
//!
//! 取数: 指数页 (index.mysteel.com/xpic/detail.html?tabName=ljm) 不用登录, 数据走
//! `getReport.ms`, 请求头要带前端同样的签名 (appKey/appSecret 是网页公开 JS 里的常量).
//! 钢铁网改了 JS 或换了密钥, 接口会回 `code: 401` —— 这时只记错误日志, 库里的旧值照用,
//! 界面按基准的日期看出指数停更了.
//!
//! 节奏: 库表空就回填一年; 之后北京时间 17:30 (当天样本截止 17:00) 以后拉最近 15 天
//! upsert, 当天成功一次就不再拉, 失败下个小时再试. 设 `MCCI_FETCH=off` 关掉.

use std::collections::HashMap;
use std::time::Duration;

use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Datelike, Days, FixedOffset, NaiveDate, NaiveTime, Utc};
use md5::{Digest, Md5};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

use super::{database_error, is_authenticated, unauthorized, AppState};
use crate::coal_index::IndexPoint;

const BASE_URL: &str = "https://index.mysteel.com";
const REPORT_PATH: &str = "/zs/newxpic/getReport.ms";
const APP_KEY: &str = "47EE3F12CF0C443F8FD51EFDA73AC815";
const APP_SECRET: &str = "3BA6477330684B19AA6AF4485497B5F2";
const API_VERSION: &str = "1.0.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// 两个品种之间歇一下, 别像爬虫一样连发.
const BETWEEN_SERIES: Duration = Duration::from_secs(2);
/// 多久醒一次看要不要拉.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// 当天样本 17:00 截止, 留半小时给发布.
const PUBLISH_AFTER: NaiveTime = match NaiveTime::from_hms_opt(17, 30, 0) {
    Some(time) => time,
    None => panic!("17:30 是合法时刻"),
};
/// 当天的值一直没出来就别再试了 (节假日).
const GIVE_UP_AFTER: NaiveTime = match NaiveTime::from_hms_opt(22, 30, 0) {
    Some(time) => time,
    None => panic!("22:30 是合法时刻"),
};
/// 平时只补最近这么多天 (覆盖周末、节假日和服务停机).
const RECENT_DAYS: u64 = 15;
/// 库表空时回填的天数 (够算一年的波动).
const BACKFILL_DAYS: u64 = 366;
/// 历史最早一天晚于"回填起点 + 这么多天"就算没回填完 (年初几天本来就是节假日).
const BACKFILL_SLACK_DAYS: u64 = 30;
/// 推算旧报价只用得着这么久的历史.
const HISTORY_DAYS: u64 = 400;

/// 要拉的一条序列: 钢铁网的品种代码与品种名 (接口两个都要).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Series {
    pub code: String,
    pub type_name: String,
}

/// master `price_factor.index_series` 里配置的序列.
pub(crate) fn configured_series(master: &Value) -> Vec<Series> {
    master["price_factor"]["index_series"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    Some(Series {
                        code: entry["code"].as_str()?.to_string(),
                        type_name: entry["type_name"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn beijing() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8 是合法时区")
}

/// 前端同款签名: MD5("path" + 路径 + "timestamp" + 毫秒时间戳 + "version" + 版本 + appSecret), 大写十六进制.
pub(crate) fn sign(path: &str, timestamp: i64) -> String {
    let digest = Md5::digest(format!(
        "path{path}timestamp{timestamp}version{API_VERSION}{APP_SECRET}"
    ));
    digest.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// 解析 `getReport.ms` 的响应为升序序列. 没数据 (`{"title":"没有数据"}`) 是空序列;
/// 接口报错 (如签名失效 `code: 401`) 或不是 JSON 返回 Err, 让调用方报警.
pub(crate) fn parse_report(body: &str) -> Result<Vec<IndexPoint>, String> {
    let report: Value =
        serde_json::from_str(body).map_err(|_| format!("响应不是 JSON: {:.80}", body))?;
    if let Some(code) = report["code"].as_i64().filter(|code| *code != 200) {
        let message = report["message"].as_str().unwrap_or("");
        return Err(format!("接口返回 {code}: {message}"));
    }
    let mut points: Vec<IndexPoint> = report["data"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let date = row["date"].as_str()?.replace('/', "-");
                    NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()?;
                    let price = row["value"].as_str()?.trim().parse::<f64>().ok()?;
                    (price.is_finite() && price > 0.0).then_some(IndexPoint { date, price })
                })
                .collect()
        })
        .unwrap_or_default();
    points.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(points)
}

/// 库里一条序列的现状 (北京时间), 调度只看它 —— 重启、多副本都不会重复拉.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stored {
    pub first: NaiveDate,
    pub latest: NaiveDate,
    /// 最近一次成功入库的时刻.
    pub fetched_at: DateTime<FixedOffset>,
}

/// 这一刻这条序列要拉哪段日期 (北京时间); None = 不用拉.
pub(crate) fn fetch_window(
    now: DateTime<FixedOffset>,
    stored: Option<Stored>,
) -> Option<(NaiveDate, NaiveDate)> {
    let today = now.date_naive();
    let backfill = Some((today - Days::new(BACKFILL_DAYS), today));
    let Some(stored) = stored else {
        return backfill;
    };
    let fetched_on = stored.fetched_at.date_naive();
    // 历史不满一年 (回填只成功了一部分、或新加的序列): 一天补一次.
    if stored.first > today - Days::new(BACKFILL_DAYS - BACKFILL_SLACK_DAYS) && fetched_on < today {
        return backfill;
    }
    if stored.latest >= today || now.time() < PUBLISH_AFTER {
        return None;
    }
    // 今天发布后已经拉过、但还没今天的值: 工作日每小时再试到 22:30, 周末不再试.
    let pulled_today = fetched_on == today && stored.fetched_at.time() >= PUBLISH_AFTER;
    if pulled_today {
        let weekday = today.weekday().number_from_monday() <= 5;
        if !weekday || now.time() >= GIVE_UP_AFTER {
            return None;
        }
    }
    Some(((today - Days::new(RECENT_DAYS)).min(stored.latest), today))
}

/// 失败第 n 次后隔多久再试: 1h, 2h, 4h… 封顶一天.
pub(crate) fn backoff(failures: u32) -> chrono::Duration {
    let hours = 1_i64 << failures.saturating_sub(1).min(5);
    chrono::Duration::hours(hours.min(24))
}

/// 两周以上的窗口一个点都没有, 说明品种代码/名字或接口坏了.
pub(crate) fn check_points(
    points: &[IndexPoint],
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), String> {
    if points.is_empty() && (end - start).num_days() >= RECENT_DAYS as i64 {
        return Err(format!(
            "{start}~{end} 一个点都没有, 品种代码或接口可能变了"
        ));
    }
    Ok(())
}

/// 拉一条序列的一段日期.
async fn fetch_series(
    client: &reqwest::Client,
    series: &Series,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<IndexPoint>, String> {
    let timestamp = Utc::now().timestamp_millis();
    let body = client
        .get(format!("{BASE_URL}{REPORT_PATH}"))
        .query(&[
            ("typeName", series.type_name.as_str()),
            ("tabName", series.code.as_str()),
            ("dateType", "day"),
            ("startTime", &start.format("%Y-%m-%d").to_string()),
            ("endTime", &end.format("%Y-%m-%d").to_string()),
            ("returnType", ""),
            ("callback", "json"),
            ("v", &timestamp.to_string()),
        ])
        .header("version", API_VERSION)
        .header("appKey", APP_KEY)
        .header("timestamp", timestamp.to_string())
        .header("sign", sign(REPORT_PATH, timestamp))
        .header(
            "Referer",
            format!("{BASE_URL}/xpic/detail.html?tabName=ljm"),
        )
        .send()
        .await
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    parse_report(&body)
}

/// 界面用的指数总览: master 配置的每条序列 (标准品指标) + 近一年的值 (升序). 库里没数据的
/// 序列也列出来 (points 为空), 界面据此提示"还没拉到".
pub(crate) fn overview(config: &[Value], history: &HashMap<String, Vec<IndexPoint>>) -> Vec<Value> {
    config
        .iter()
        .filter_map(|series| {
            let code = series["code"].as_str()?;
            let mut row = series.clone();
            row["points"] = json!(history.get(code).cloned().unwrap_or_default());
            Some(row)
        })
        .collect()
}

/// GET /api/mcci —— 钢铁网炼焦煤指数总览 (登录即可). 只读库, 不碰钢铁网.
pub(crate) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }
    // 指数序列不在煤库覆盖层里, 读内置 master 即可 (与 spawn_daily 同一份), 不用合并覆盖层.
    let master: Value = serde_json::from_str(blend_kit::master_json()).unwrap_or_default();
    let config = master["price_factor"]["index_series"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let history = match &state.database {
        Some(pool) => match history(pool).await {
            Ok(history) => history,
            Err(error) => return database_error(error),
        },
        None => HashMap::new(),
    };
    Json(json!({ "ok": true, "series": overview(&config, &history) })).into_response()
}

/// 一条序列一条语句写完 (回填一年也只一次往返).
async fn upsert(pool: &PgPool, code: &str, points: &[IndexPoint]) -> Result<(), sqlx::Error> {
    let dates: Vec<NaiveDate> = points
        .iter()
        .filter_map(|point| NaiveDate::parse_from_str(&point.date, "%Y-%m-%d").ok())
        .collect();
    let values: Vec<f64> = points.iter().map(|point| point.price).collect();
    sqlx::query(
        r#"
        INSERT INTO mcci_daily (series_code, trade_date, value)
        SELECT $1, d, v FROM UNNEST($2::date[], $3::float8[]) AS t(d, v)
        ON CONFLICT (series_code, trade_date)
        DO UPDATE SET value = EXCLUDED.value, fetched_at = NOW()
        "#,
    )
    .bind(code)
    .bind(&dates)
    .bind(&values)
    .execute(pool)
    .await?;
    Ok(())
}

/// 库里每条序列的现状.
async fn stored_state(pool: &PgPool) -> Result<HashMap<String, Stored>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT series_code, MIN(trade_date) AS first, MAX(trade_date) AS latest,
               MAX(fetched_at) AS fetched_at
        FROM mcci_daily GROUP BY series_code
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let fetched_at: DateTime<Utc> = row.try_get("fetched_at")?;
            Ok((
                row.try_get("series_code")?,
                Stored {
                    first: row.try_get("first")?,
                    latest: row.try_get("latest")?,
                    fetched_at: fetched_at.with_timezone(&beijing()),
                },
            ))
        })
        .collect()
}

/// 每条序列近一年多的历史 (升序), 给估价当基准和推算旧报价.
pub(crate) async fn history(
    pool: &PgPool,
) -> Result<HashMap<String, Vec<IndexPoint>>, sqlx::Error> {
    let since = Utc::now().with_timezone(&beijing()).date_naive() - Days::new(HISTORY_DAYS);
    let rows = sqlx::query(
        r#"
        SELECT series_code, trade_date, value FROM mcci_daily
        WHERE trade_date >= $1
        ORDER BY series_code, trade_date
        "#,
    )
    .bind(since)
    .fetch_all(pool)
    .await?;
    let mut series: HashMap<String, Vec<IndexPoint>> = HashMap::new();
    for row in rows {
        let date: NaiveDate = row.try_get("trade_date")?;
        series
            .entry(row.try_get("series_code")?)
            .or_default()
            .push(IndexPoint {
                date: date.format("%Y-%m-%d").to_string(),
                price: row.try_get("value")?,
            });
    }
    Ok(series)
}

/// 拉一条序列并入库, 返回入库条数.
async fn refresh(
    pool: &PgPool,
    client: &reqwest::Client,
    series: &Series,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<usize, String> {
    let points = fetch_series(client, series, start, end).await?;
    check_points(&points, start, end)?;
    upsert(pool, &series.code, &points)
        .await
        .map_err(|error| error.to_string())?;
    Ok(points.len())
}

/// 某条序列连续失败的次数与下次可以再试的时刻 (只在内存: 重启后先试一次无妨).
#[derive(Default)]
struct Failure {
    count: u32,
    retry_at: Option<DateTime<FixedOffset>>,
}

/// 一轮: 逐条序列看要不要拉; 失败的按 [`backoff`] 隔开再试.
async fn tick(
    pool: &PgPool,
    client: &reqwest::Client,
    all: &[Series],
    failures: &mut HashMap<String, Failure>,
) {
    let state = match stored_state(pool).await {
        Ok(state) => state,
        Err(error) => {
            tracing::error!(%error, "读取钢铁网指数库表失败");
            return;
        }
    };
    let mut fetched_any = false;
    for series in all {
        let now = Utc::now().with_timezone(&beijing());
        let failure = failures.entry(series.code.clone()).or_default();
        if failure.retry_at.is_some_and(|at| now < at) {
            continue;
        }
        let Some((start, end)) = fetch_window(now, state.get(&series.code).copied()) else {
            continue;
        };
        if fetched_any {
            tokio::time::sleep(BETWEEN_SERIES).await;
        }
        fetched_any = true;
        match refresh(pool, client, series, start, end).await {
            Ok(count) => {
                *failure = Failure::default();
                tracing::info!(code = %series.code, %start, %end, count, "已更新钢铁网炼焦煤指数");
            }
            Err(error) => {
                failure.count += 1;
                let wait = backoff(failure.count);
                failure.retry_at = Some(now + wait);
                tracing::error!(
                    code = %series.code, %error, failures = failure.count, retry_in_hours = wait.num_hours(),
                    "拉钢铁网炼焦煤指数失败"
                );
            }
        }
    }
}

/// 起后台任务, 每小时醒一次逐条序列按 [`fetch_window`] 决定拉不拉. 每一轮在子任务里跑,
/// 万一 panic 只丢这一轮、记日志, 不让取数悄悄停掉. 配了 `MCCI_FETCH=off` 或 master
/// 没配序列就不起.
pub(crate) fn spawn_daily(pool: PgPool, master: &Value) {
    if std::env::var("MCCI_FETCH").is_ok_and(|value| value == "off") {
        tracing::info!("MCCI_FETCH=off, 不拉钢铁网炼焦煤指数");
        return;
    }
    let all = configured_series(master);
    if all.is_empty() {
        return;
    }
    let client = match reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("Mozilla/5.0 (compatible; doudou-blend/1.0)")
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "建 HTTP 客户端失败, 不拉钢铁网炼焦煤指数");
            return;
        }
    };
    tokio::spawn(async move {
        let failures = std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let mut ticker = tokio::time::interval(CHECK_EVERY);
        loop {
            ticker.tick().await;
            let (pool, client, all, failures) =
                (pool.clone(), client.clone(), all.clone(), failures.clone());
            let round = tokio::spawn(async move {
                let mut failures = failures.lock().await;
                tick(&pool, &client, &all, &mut failures).await;
            });
            if let Err(error) = round.await {
                tracing::error!(%error, "钢铁网指数取数这一轮异常退出, 下个小时再来");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(date: &str, time: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_str(&format!("{date} {time} +0800"), "%Y-%m-%d %H:%M %z").unwrap()
    }

    fn day(date: &str) -> NaiveDate {
        NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn test_sign_matches_web_frontend() {
        // 与网页 sign.js 对同一输入算出的值一致.
        assert_eq!(
            sign("/zs/newxpic/getReport.ms", 1_793_064_998_408),
            "E953BF05ADC059216F278422D2E921E0"
        );
    }

    #[test]
    fn test_parse_report_sorts_and_normalizes_dates() {
        let body = r#"{"dateType":"day","data":[
            {"date":"2026/10/26","value":"2035.50","lastdayValue":"2040.30"},
            {"date":"2026/10/23","value":"2040.30"},
            {"date":"2026/10/22","value":"--"}
        ]}"#;
        let points = parse_report(body).unwrap();
        assert_eq!(
            points,
            vec![
                IndexPoint {
                    date: "2026-10-23".into(),
                    price: 2040.3
                },
                IndexPoint {
                    date: "2026-10-26".into(),
                    price: 2035.5
                },
            ]
        );
    }

    #[test]
    fn test_parse_report_no_data_is_empty_but_errors_are_errors() {
        assert!(parse_report(r#"{"title":"没有数据"}"#).unwrap().is_empty());
        let unauthorized =
            parse_report(r#"{"code":401,"message":"认证参数传入不完整!","data":null}"#);
        assert!(unauthorized.unwrap_err().contains("认证参数"));
        assert!(parse_report("<html>").is_err());
    }

    /// 库里一条序列: 一年历史齐全, 最新到 `latest`, 上次拉取在 `fetched`.
    fn stored(latest: &str, fetched: (&str, &str)) -> Option<Stored> {
        Some(Stored {
            first: day("2025-10-27"),
            latest: day(latest),
            fetched_at: at(fetched.0, fetched.1),
        })
    }

    // 2026-10-27 是周二, 10-31 是周六.

    #[test]
    fn test_fetch_window_backfills_new_series_any_time() {
        assert_eq!(
            fetch_window(at("2026-10-27", "09:00"), None),
            Some((day("2025-10-26"), day("2026-10-27")))
        );
    }

    #[test]
    fn test_fetch_window_backfills_incomplete_history_once_a_day() {
        // 回填只成功了一半: 只有近两周. 今天还没拉过 → 补一年; 今天拉过 → 按平时的规则.
        let partial = |fetched: (&str, &str)| {
            Some(Stored {
                first: day("2026-10-12"),
                latest: day("2026-10-26"),
                fetched_at: at(fetched.0, fetched.1),
            })
        };
        assert_eq!(
            fetch_window(at("2026-10-27", "09:00"), partial(("2026-10-26", "18:00"))),
            Some((day("2025-10-26"), day("2026-10-27")))
        );
        assert_eq!(
            fetch_window(at("2026-10-27", "09:00"), partial(("2026-10-27", "08:00"))),
            None
        );
    }

    #[test]
    fn test_fetch_window_waits_for_publish() {
        let yesterday = stored("2026-10-26", ("2026-10-26", "18:00"));
        assert_eq!(fetch_window(at("2026-10-27", "17:29"), yesterday), None);
        assert_eq!(
            fetch_window(at("2026-10-27", "17:30"), yesterday),
            Some((day("2026-10-12"), day("2026-10-27")))
        );
    }

    #[test]
    fn test_fetch_window_stops_once_today_is_in() {
        let today = stored("2026-10-27", ("2026-10-27", "17:31"));
        assert_eq!(fetch_window(at("2026-10-27", "18:30"), today), None);
    }

    #[test]
    fn test_fetch_window_retries_weekday_until_today_shows_up() {
        // 17:30 拉过但今天的值还没发布: 工作日每小时再试, 22:30 后放弃 (节假日).
        let pulled = stored("2026-10-26", ("2026-10-27", "17:30"));
        assert!(fetch_window(at("2026-10-27", "20:00"), pulled).is_some());
        assert_eq!(fetch_window(at("2026-10-27", "22:30"), pulled), None);
    }

    #[test]
    fn test_fetch_window_weekend_pulls_once() {
        // 周六: 第一次照拉 (补周五可能漏的), 拉过就不再试.
        let friday = stored("2026-10-30", ("2026-10-30", "18:00"));
        assert!(fetch_window(at("2026-10-31", "18:00"), friday).is_some());
        let pulled = stored("2026-10-30", ("2026-10-31", "17:30"));
        assert_eq!(fetch_window(at("2026-10-31", "19:00"), pulled), None);
    }

    #[test]
    fn test_fetch_window_reaches_back_past_a_long_outage() {
        // 停了一个月: 从库里最新那天补起.
        assert_eq!(
            fetch_window(
                at("2026-10-27", "18:00"),
                stored("2026-09-20", ("2026-09-20", "18:00"))
            ),
            Some((day("2026-09-20"), day("2026-10-27")))
        );
    }

    #[test]
    fn test_backoff_doubles_up_to_a_day() {
        let hours = |n| backoff(n).num_hours();
        assert_eq!([1, 2, 3, 5, 6, 30].map(hours), [1, 2, 4, 16, 24, 24]);
    }

    #[test]
    fn test_check_points_rejects_empty_answer_over_long_window() {
        // 两周以上一个点都没有, 说明接口参数坏了, 不能当成功.
        assert!(check_points(&[], day("2026-10-12"), day("2026-10-27")).is_err());
        // 窗口很短 (假期) 没数据是正常的.
        assert!(check_points(&[], day("2026-10-25"), day("2026-10-27")).is_ok());
    }

    #[test]
    fn test_overview_lists_every_configured_series_with_its_points() {
        let config = vec![
            json!({"code": "LJM_FM", "type_name": "肥煤", "name": "肥煤指数",
                   "coal_type": "肥煤", "form": "精煤", "region": "山西",
                   "props": {"A": 10, "V": 30, "S": 1.8, "G": 90, "M": 10}}),
            json!({"code": "LJM_QM", "type_name": "气煤", "name": "气煤指数",
                   "coal_type": "气煤", "props": {"S": 0.6}}),
        ];
        let history = HashMap::from([(
            "LJM_FM".to_string(),
            vec![
                IndexPoint {
                    date: "2026-10-23".into(),
                    price: 2188.7,
                },
                IndexPoint {
                    date: "2026-10-26".into(),
                    price: 2190.0,
                },
            ],
        )]);
        let rows = overview(&config, &history);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["code"], "LJM_FM");
        assert_eq!(rows[0]["name"], "肥煤指数");
        assert_eq!(rows[0]["type_name"], "肥煤");
        assert_eq!(rows[0]["coal_type"], "肥煤");
        assert_eq!(rows[0]["props"]["G"], 90);
        assert_eq!(
            rows[0]["points"][1],
            json!({"date": "2026-10-26", "price": 2190.0})
        );
        // 还没拉到的序列照样列出, points 为空.
        assert_eq!(rows[1]["code"], "LJM_QM");
        assert_eq!(rows[1]["points"], json!([]));
    }

    /// 连钢铁网实拉一次 (默认不跑): 怀疑签名失效时 `cargo test mcci_live -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn test_mcci_live_fetch_every_configured_series() {
        let master: Value = serde_json::from_str(blend_kit::master_json()).unwrap();
        let all = configured_series(&master);
        assert!(!all.is_empty(), "master 没配 index_series");
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap();
        let today = Utc::now().with_timezone(&beijing()).date_naive();
        for (i, series) in all.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(BETWEEN_SERIES).await;
            }
            let points = fetch_series(&client, series, today - Days::new(14), today)
                .await
                .unwrap_or_else(|error| panic!("{}: {error}", series.code));
            let last = points
                .last()
                .unwrap_or_else(|| panic!("{} 两周没数据", series.code));
            println!(
                "{} {} {} {}",
                series.code, series.type_name, last.date, last.price
            );
        }
    }

    #[test]
    fn test_configured_series_reads_master_and_skips_incomplete() {
        let master = json!({"price_factor": {"index_series": [
            {"code": "LJM_DLZJM", "type_name": "低硫主焦煤", "coal_type": "焦煤"},
            {"code": "LJM_QM"},
            {"type_name": "瘦煤"}
        ]}});
        assert_eq!(
            configured_series(&master),
            vec![Series {
                code: "LJM_DLZJM".into(),
                type_name: "低硫主焦煤".into()
            }]
        );
        assert!(configured_series(&json!({})).is_empty());
    }
}
