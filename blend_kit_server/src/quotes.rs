//! 询价按批存档与估价对比; 不进煤池、不跨次沿用, 估价只由服务端重算.

use super::benchmarks;
use super::{
    authenticated_username, bad_request, database_error, mcci, merged_master_json, require_admin,
    require_database, require_json_content_type, AppState,
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use blend_kit::INDICATORS;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

const MAX_QUOTE_BODY: usize = 256 * 1024;
const MAX_ITEMS: usize = 50;
const MAX_NAME_CHARS: usize = 100;
const MAX_FOB: f64 = 100_000.0;
const MAX_FRT: f64 = 10_000.0;

/// 一次精算的询价. 不拒收未知字段: 浏览器顺手带上的估价之类一律忽略, 估价只由服务端算.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QuoteBatch {
    pub batch_id: String,
    pub items: Vec<QuoteItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QuoteItem {
    pub name: String,
    /// 出厂价 (含税), 元/吨.
    pub fob: f64,
    /// 运费, 元/吨; 不填记 0.
    #[serde(default)]
    pub frt: f64,
    #[serde(default)]
    pub coal_type: Option<String>,
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    /// 这批实际用的指标 (煤池值 ∪ 当场改动).
    pub props: Map<String, Value>,
    /// 当场改过的指标名.
    #[serde(default)]
    pub changed: Vec<String>,
}

fn validate(batch: &QuoteBatch) -> Result<(), String> {
    let id = &batch.batch_id;
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("batch_id 须为 1–64 位字母、数字或连字符".into());
    }
    if batch.items.is_empty() || batch.items.len() > MAX_ITEMS {
        return Err(format!("询价须为 1–{MAX_ITEMS} 条"));
    }
    let mut names = HashSet::new();
    for item in &batch.items {
        let name = item.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("煤名不能为空, 且不超过 {MAX_NAME_CHARS} 字"));
        }
        if !names.insert(name) {
            return Err(format!("煤名重复: {name}"));
        }
        if !(item.fob.is_finite() && item.fob > 0.0 && item.fob < MAX_FOB) {
            return Err(format!("{name} 的出厂价须在 0–{MAX_FOB:.0} 元之间"));
        }
        if !(item.frt.is_finite() && item.frt >= 0.0 && item.frt < MAX_FRT) {
            return Err(format!("{name} 的运费须在 0–{MAX_FRT:.0} 元之间"));
        }
        if let Some(bad) = item
            .changed
            .iter()
            .find(|key| !INDICATORS.contains(&key.as_str()))
        {
            return Err(format!("{name} 的改动指标 {bad} 不认识"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Estimate {
    /// 同口径 (出厂) 估价; 没对上基准为 None.
    fair: Option<f64>,
    points: Option<f64>,
    benchmark: Option<String>,
}

/// 按线上当前系数表与基准估这批煤的出厂价. 只认按基准估出来的价: 没对上基准时核心会拿
/// 这批报价自己反推市场价, 拿它和报价比是循环论证.
fn estimate_items(
    items: &[QuoteItem],
    master: &Value,
    benchmarks: &[Value],
) -> Result<Vec<Estimate>, String> {
    let mut request = json!({ "coals": items.iter().map(price_coal).collect::<Vec<_>>() });
    benchmarks::fill_price_request(&mut request, master, benchmarks);
    let output: Value = serde_json::from_str(&blend_kit::estimate_json(&request.to_string()))
        .map_err(|error| format!("估价结果解析失败: {error}"))?;
    if output["ok"] != json!(true) {
        return Err(output["reason"].as_str().unwrap_or("估价失败").to_string());
    }
    let values = output["coal_values"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .map(|item| {
            let Some(value) = values
                .iter()
                .find(|value| value["name"] == item.name.as_str())
            else {
                return Estimate::default();
            };
            let benchmark = value["benchmark"].as_str().map(str::to_string);
            Estimate {
                fair: value["fair"]
                    .as_f64()
                    .filter(|fair| fair.is_finite() && *fair > 0.0 && benchmark.is_some()),
                points: value["points"].as_f64(),
                benchmark,
            }
        })
        .collect())
}

/// 估价用的煤: 出厂口径 (运费不进), 空值指标去掉 (同每日快照).
fn price_coal(item: &QuoteItem) -> Value {
    let props: Map<String, Value> = item
        .props
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut coal = json!({ "name": item.name, "props": props, "fob": item.fob, "frt": 0 });
    for (key, value) in [
        ("coal_type", &item.coal_type),
        ("form", &item.form),
        ("region", &item.region),
    ] {
        if let Some(value) = value {
            coal[key] = json!(value);
        }
    }
    coal
}

/// 存档后回给精算页的一条: 报价与服务端估价的对比.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchivedQuote {
    pub name: String,
    pub fob: f64,
    pub estimate: Option<f64>,
    /// fob − estimate.
    pub gap: Option<f64>,
    /// fob / estimate.
    pub ratio: Option<f64>,
    pub benchmark: Option<String>,
}

/// 本账号某种煤最近一次询价, 只作精算页输入框的占位提示.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct LatestQuote {
    pub name: String,
    pub fob: f64,
    pub frt: f64,
    pub quoted_on: NaiveDate,
}

/// 按批覆盖: 同一 (账号, 批) 先删后插, 一个事务, 失败不留半批.
async fn replace_batch(
    pool: &PgPool,
    username: &str,
    day: NaiveDate,
    batch: &QuoteBatch,
    estimates: &[Estimate],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // 先串行化同批写入, 下一条 DELETE 才能看到上一事务刚提交的整批数据.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))")
        .bind(username)
        .bind(&batch.batch_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM coal_quotes WHERE username = $1 AND batch_id = $2")
        .bind(username)
        .bind(&batch.batch_id)
        .execute(&mut *tx)
        .await?;
    for (item, estimate) in batch.items.iter().zip(estimates) {
        sqlx::query(
            r#"
            INSERT INTO coal_quotes
                (id, username, batch_id, quoted_on, name, coal_type, form, region,
                 fob, frt, props, changed, estimate, points, benchmark)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(username)
        .bind(&batch.batch_id)
        .bind(day)
        .bind(&item.name)
        .bind(&item.coal_type)
        .bind(&item.form)
        .bind(&item.region)
        .bind(item.fob)
        .bind(item.frt)
        .bind(Value::Object(item.props.clone()))
        .bind(json!(item.changed))
        .bind(estimate.fair)
        .bind(estimate.points)
        .bind(&estimate.benchmark)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// POST /api/quotes —— 存档一批询价 (登录即可). 估价由这里重算, 不信任浏览器.
pub(crate) async fn archive(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    if let Err(response) = require_json_content_type(&headers) {
        return *response;
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_QUOTE_BODY).await else {
        return bad_request("请求体读取失败或超出大小上限");
    };
    let batch: QuoteBatch = match serde_json::from_slice(&body) {
        Ok(batch) => batch,
        Err(error) => return bad_request(&format!("询价格式无效: {error}")),
    };
    if let Err(reason) = validate(&batch) {
        return bad_request(&reason);
    }
    let master: Value = serde_json::from_str(&merged_master_json(&state).await).unwrap_or_default();
    let benchmarks = benchmarks::current(&state, &master).await;
    let estimates = match estimate_items(&batch.items, &master, &benchmarks) {
        Ok(estimates) => estimates,
        Err(reason) => return bad_request(&reason),
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    let day = Utc::now().with_timezone(&mcci::beijing()).date_naive();
    match replace_batch(pool, &username, day, &batch, &estimates).await {
        Ok(()) => {
            Json(json!({ "ok": true, "items": archived(&batch.items, &estimates) })).into_response()
        }
        Err(error) => database_error(error),
    }
}

fn archived(items: &[QuoteItem], estimates: &[Estimate]) -> Vec<ArchivedQuote> {
    items
        .iter()
        .zip(estimates)
        .map(|(item, estimate)| ArchivedQuote {
            name: item.name.clone(),
            fob: item.fob,
            estimate: estimate.fair,
            gap: estimate.fair.map(|fair| item.fob - fair),
            ratio: estimate.fair.map(|fair| item.fob / fair),
            benchmark: estimate.benchmark.clone(),
        })
        .collect()
}

/// 本账号每种煤最近一次询价, 只供输入框占位提示.
pub(crate) async fn latest(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match sqlx::query_as::<_, LatestQuote>(
        "SELECT DISTINCT ON (name) name, fob, frt, quoted_on FROM coal_quotes WHERE username = $1 ORDER BY name, created_at DESC"
    ).bind(username).fetch_all(pool).await {
        Ok(quotes) => Json(json!({ "ok": true, "quotes": quotes })).into_response(),
        Err(error) => database_error(error),
    }
}

const DEFAULT_LIMIT: i64 = 200;
const MAX_LIMIT: i64 = 1000;
const UNTYPED: &str = "未标煤种";

/// 后台明细一行.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct QuoteRecord {
    pub id: String,
    pub username: String,
    pub batch_id: String,
    pub quoted_on: NaiveDate,
    pub name: String,
    pub coal_type: Option<String>,
    pub form: Option<String>,
    pub region: Option<String>,
    pub fob: f64,
    pub frt: f64,
    #[sqlx(json)]
    pub props: Map<String, Value>,
    #[sqlx(json)]
    pub changed: Vec<String>,
    pub estimate: Option<f64>,
    pub points: Option<f64>,
    pub benchmark: Option<String>,
    pub created_at: DateTime<Utc>,
    #[sqlx(skip)]
    pub gap: Option<f64>,
    #[sqlx(skip)]
    pub ratio: Option<f64>,
}

/// 按煤种汇总 "报价 / 估价": 取中位数, 个别手误带不偏.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuoteTypeSummary {
    pub coal_type: String,
    pub count: usize,
    pub median_ratio: f64,
    pub median_gap: f64,
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn summarize(rows: &[QuoteRecord]) -> Vec<QuoteTypeSummary> {
    let mut groups: BTreeMap<&str, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for row in rows {
        let Some(estimate) = row
            .estimate
            .filter(|value| value.is_finite() && *value > 0.0)
        else {
            continue;
        };
        let coal_type = row
            .coal_type
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(UNTYPED);
        let (ratios, gaps) = groups.entry(coal_type).or_default();
        ratios.push(row.fob / estimate);
        gaps.push(row.fob - estimate);
    }
    let mut summary: Vec<_> = groups
        .into_iter()
        .map(|(coal_type, (mut ratios, mut gaps))| QuoteTypeSummary {
            coal_type: coal_type.into(),
            count: ratios.len(),
            median_ratio: median(&mut ratios),
            median_gap: median(&mut gaps),
        })
        .collect();
    summary.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.coal_type.cmp(&b.coal_type))
    });
    summary
}

fn parse_limit(value: Option<&str>) -> Result<i64, &'static str> {
    match value {
        None => Ok(DEFAULT_LIMIT),
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(|n| n.min(MAX_LIMIT))
            .ok_or("limit 须为正整数"),
    }
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    limit: Option<String>,
}

pub(crate) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<ListQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    let limit = match query
        .ok()
        .and_then(|Query(query)| parse_limit(query.limit.as_deref()).ok())
    {
        Some(limit) => limit,
        None => return bad_request("limit 须为正整数"),
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    let mut quotes = match sqlx::query_as::<_, QuoteRecord>(
        "SELECT * FROM coal_quotes ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    for row in &mut quotes {
        row.gap = row.estimate.map(|estimate| row.fob - estimate);
        row.ratio = row
            .estimate
            .filter(|estimate| *estimate > 0.0)
            .map(|estimate| row.fob / estimate);
    }
    // 汇总独立查询全部样本, 不受明细 limit 截断.
    match sqlx::query_as::<_, QuoteRecord>("SELECT * FROM coal_quotes WHERE estimate > 0")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => Json(json!({ "ok": true, "quotes": quotes, "by_type": summarize(&rows) }))
            .into_response(),
        Err(error) => database_error(error),
    }
}

pub(crate) async fn delete(
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
    if Uuid::parse_str(&id).is_err() {
        return not_found();
    }
    match sqlx::query("DELETE FROM coal_quotes WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
    {
        Ok(result) if result.rows_affected() == 0 => not_found(),
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(error) => database_error(error),
    }
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "reason": "询价不存在" })),
    )
        .into_response()
}

#[cfg(test)]
#[path = "quotes_db_tests.rs"]
mod db_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn item(name: &str, fob: f64) -> QuoteItem {
        QuoteItem {
            name: name.into(),
            fob,
            frt: 0.0,
            coal_type: Some("主焦煤".into()),
            form: None,
            region: None,
            props: serde_json::from_value(json!({ "S": 0.6, "A": 9.8 })).unwrap(),
            changed: vec![],
        }
    }

    fn batch(items: Vec<QuoteItem>) -> QuoteBatch {
        QuoteBatch {
            batch_id: "0b6f4c1e-1d7a-4c55-9a51-2f0f6f3f1a10".into(),
            items,
        }
    }

    #[test]
    fn test_validate_accepts_a_normal_batch() {
        assert_eq!(
            validate(&batch(vec![item("临北", 2150.0), item("沙曲", 1980.0)])),
            Ok(())
        );
    }

    #[test]
    fn test_validate_rejects_bad_batches() {
        let mut bad_id = batch(vec![item("临北", 2150.0)]);
        bad_id.batch_id = "../x".into();
        let mut bad_frt = item("临北", 2150.0);
        bad_frt.frt = -1.0;
        let mut bad_changed = item("临北", 2150.0);
        bad_changed.changed = vec!["X".into()];
        for (case, expect) in [
            (bad_id, "batch_id"),
            (batch(vec![]), "1–50"),
            (
                batch((0..51).map(|i| item(&format!("煤{i}"), 2000.0)).collect()),
                "1–50",
            ),
            (
                batch(vec![item("临北", 2150.0), item("临北", 2100.0)]),
                "重复",
            ),
            (batch(vec![item("  ", 2150.0)]), "煤名"),
            (batch(vec![item("临北", 0.0)]), "出厂价"),
            (batch(vec![item("临北", f64::NAN)]), "出厂价"),
            (batch(vec![item("临北", 100_000.0)]), "出厂价"),
            (batch(vec![bad_frt]), "运费"),
            (batch(vec![bad_changed]), "X"),
        ] {
            let error = validate(&case).unwrap_err();
            assert!(error.contains(expect), "{error}");
        }
    }

    #[test]
    fn test_batch_ignores_client_estimate_fields() {
        let parsed: QuoteBatch = serde_json::from_value(json!({
            "batch_id": "abc-1",
            "items": [{ "name": "临北", "fob": 2150, "props": {}, "estimate": 1, "gap": 2 }]
        }))
        .unwrap();
        assert_eq!(parsed.items[0].frt, 0.0);
    }

    fn fixture_master() -> Value {
        json!({ "price_factor": {
            "adjusts": [{"indicator": "S", "direction": "Upper", "base": 0.6,
                         "premium_rate": 10.0, "discount_rate": 10.0}],
            "benchmarks": [{"name": "测试主焦", "coal_type": "主焦煤", "props": {"S": 0.6},
                            "price": 2000.0, "date": "2026-10-30"}]
        }})
    }
    fn fixture_master_without_benchmarks() -> Value {
        let mut master = fixture_master();
        master["price_factor"]["benchmarks"] = json!([]);
        master
    }
    #[test]
    fn test_estimate_items_prices_each_quote_from_benchmarks() {
        let master = fixture_master(); // 一条基准: 主焦煤, S 0.6 → 2000 元
        let benchmarks = vec![]; // 空 = 用 master 自带基准 (fill_price_request 的既有行为)
        let estimates = estimate_items(&[item("临北", 2150.0)], &master, &benchmarks).unwrap();
        assert_eq!(estimates.len(), 1);
        let fair = estimates[0].fair.unwrap();
        assert!((fair - 2000.0).abs() < 1e-6, "{fair}"); // 指标与基准一致 → 估价 = 基准价
        assert_eq!(estimates[0].benchmark.as_deref(), Some("测试主焦"));
    }

    #[test]
    fn test_estimate_ignores_freight() {
        let master = fixture_master();
        let mut with_frt = item("临北", 2150.0);
        with_frt.frt = 300.0;
        let a = estimate_items(&[item("临北", 2150.0)], &master, &[]).unwrap();
        let b = estimate_items(&[with_frt], &master, &[]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_estimate_without_benchmark_is_none() {
        // master 里没有任何基准 (也没有 market_price): 核心会拿这批报价反推市场价, 不能当估价.
        let estimates = estimate_items(
            &[item("临北", 2150.0)],
            &fixture_master_without_benchmarks(),
            &[],
        )
        .unwrap();
        assert_eq!(estimates[0].fair, None);
    }

    #[test]
    fn test_estimate_reports_core_validation_errors() {
        let mut bad = item("临北", 2150.0);
        bad.props.insert("A".into(), json!(-5)); // 负灰分触发核心指标校验.
        assert!(estimate_items(&[bad], &fixture_master(), &[]).is_err());
    }

    #[test]
    fn test_archived_quotes_compare_only_valid_estimates() {
        let items = vec![item("甲", 2200.0), item("乙", 1900.0), item("丙", 1800.0)];
        let estimates = vec![
            Estimate {
                fair: Some(2000.0),
                ..Estimate::default()
            },
            Estimate {
                fair: Some(2000.0),
                ..Estimate::default()
            },
            Estimate::default(),
        ];
        let rows = archived(&items, &estimates);
        assert_eq!((rows[0].gap, rows[0].ratio), (Some(200.0), Some(1.1)));
        assert_eq!((rows[1].gap, rows[1].ratio), (Some(-100.0), Some(0.95)));
        assert_eq!(
            (rows[2].estimate, rows[2].gap, rows[2].ratio),
            (None, None, None)
        );
    }

    fn record(coal_type: Option<&str>, fob: f64, estimate: Option<f64>) -> QuoteRecord {
        QuoteRecord {
            id: "id".into(),
            username: "tester".into(),
            batch_id: "batch".into(),
            quoted_on: "2026-10-31".parse().unwrap(),
            name: "测试煤".into(),
            coal_type: coal_type.map(str::to_string),
            form: None,
            region: None,
            fob,
            frt: 0.0,
            props: Map::new(),
            changed: vec![],
            estimate,
            points: None,
            benchmark: None,
            created_at: "2026-10-31T00:00:00Z".parse().unwrap(),
            gap: None,
            ratio: None,
        }
    }
    #[test]
    fn test_summarize_groups_by_type_with_medians() {
        let rows = vec![
            record(Some("主焦煤"), 2200.0, Some(2000.0)), // 1.10, +200
            record(Some("主焦煤"), 2100.0, Some(2000.0)), // 1.05, +100
            record(Some("主焦煤"), 2400.0, Some(2000.0)), // 1.20, +400
            record(Some("肥煤"), 1900.0, Some(2000.0)),   // 0.95, -100
            record(Some("肥煤"), 2100.0, Some(2000.0)),   // 1.05, +100
            record(None, 1500.0, Some(1500.0)),
            record(Some("肥煤"), 1800.0, None), // 没估价, 不进汇总
        ];
        let summary = summarize(&rows);
        assert_eq!(summary[0].coal_type, "主焦煤");
        assert_eq!(summary[0].count, 3);
        assert!((summary[0].median_ratio - 1.10).abs() < 1e-9);
        assert!((summary[0].median_gap - 200.0).abs() < 1e-9);
        assert_eq!(summary[1].coal_type, "肥煤");
        assert_eq!(summary[1].count, 2);
        assert!((summary[1].median_ratio - 1.0).abs() < 1e-9); // 偶数取中间两个的平均
        assert!((summary[1].median_gap - 0.0).abs() < 1e-9);
        assert_eq!(summary[2].coal_type, "未标煤种");
    }

    #[test]
    fn test_summarize_quotes_handles_empty_types_and_missing_estimates() {
        let rows = vec![
            record(Some("  "), 2200.0, Some(2000.0)),
            record(None, 1800.0, Some(2000.0)),
            record(Some("甲"), 2000.0, Some(0.0)),
            record(Some("乙"), 2000.0, None),
        ];
        let result = summarize(&rows);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].coal_type, "未标煤种");
        assert_eq!(result[0].count, 2);
        assert_eq!(result[0].median_gap, 0.0);
        assert!(summarize(&[]).is_empty());
    }
    #[test]
    fn test_quote_limit_defaults_caps_and_rejects_invalid() {
        assert_eq!(parse_limit(None), Ok(200));
        assert_eq!(parse_limit(Some("1001")), Ok(1000));
        for value in ["", "abc", "0", "-1", "1.5"] {
            assert!(parse_limit(Some(value)).is_err());
        }
    }
}
