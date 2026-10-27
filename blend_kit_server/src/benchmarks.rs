//! 基准报价 (分煤种的市场价, 带报价日): 估价模块 `blend_kit::pricing` 按它定价.
//!
//! 两层来源, 同名取报价日最新的一条 (同日以库表为准):
//! - `master.price_factor.benchmarks` —— 随版本发布的默认值 (整理自公开报价);
//! - 库表 `price_benchmarks` —— 管理员随时补录, 不用重新部署; 旧报价留着供回测.
//!
//! 报价日早于焦煤现货指数 (CCP) 最新一期时, 按指数涨跌把价推算到今天 (`current_price`),
//! 原价、比例与指数期都保留给界面展示. CCP 是指数点 (2017-01 = 1000), 只用来算涨跌比例,
//! 不当价格用.

use std::collections::HashMap;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::NaiveDate;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{
    bad_request, database_error, is_authenticated, merged_master_json, require_admin,
    require_database, require_json_content_type, unauthorized, AppState,
};
use crate::coal_index::{self, IndexPoint};
use crate::mcci;

/// 只给人看或只给服务端用的价格因子字段, 交给核心前去掉.
pub(crate) const MASTER_ONLY_KEYS: [&str; 5] = [
    "description",
    "uncertainty_note",
    "benchmarks_note",
    "index_series",
    "index_series_note",
];

/// 一条基准报价的请求体上限.
const MAX_BENCHMARK_BODY: usize = 16 * 1024;
/// 估价请求体上限 (与求解同量级: 一百多种煤).
const MAX_PRICE_BODY: usize = 1024 * 1024;

/// 同名取报价日最新的一条; 后面的层 (库表) 同日覆盖前面的 (master). 按煤种、名字排序,
/// 估价模块遇到并列时取先列的, 顺序要稳定.
pub(crate) fn merge(layers: &[&[Value]]) -> Vec<Value> {
    let mut latest: HashMap<String, Value> = HashMap::new();
    for layer in layers {
        for benchmark in *layer {
            let (Some(name), Some(date)) = (benchmark["name"].as_str(), benchmark["date"].as_str())
            else {
                continue;
            };
            let newer = latest
                .get(name)
                .and_then(|kept| kept["date"].as_str())
                .is_none_or(|kept| date >= kept);
            if newer {
                latest.insert(name.to_string(), benchmark.clone());
            }
        }
    }
    let mut merged: Vec<Value> = latest.into_values().collect();
    merged.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["coal_type"].as_str().unwrap_or("").to_string(),
                v["name"].as_str().unwrap_or("").to_string(),
            )
        };
        key(a).cmp(&key(b))
    });
    merged
}

/// 报价日到指数最新一期的涨跌比例. 报价日之后没有新一期 (或报价日早于整条序列) 就不推算.
pub(crate) fn index_ratio(series: &[IndexPoint], date: &str) -> Option<(f64, String)> {
    let latest = series.last()?;
    if latest.date.as_str() <= date {
        return None;
    }
    let base = series
        .iter()
        .rev()
        .find(|point| point.date.as_str() <= date)?;
    (base.price > 0.0).then(|| (latest.price / base.price, latest.date.clone()))
}

/// 给报价日较旧的基准填 current_price (按 `pick` 给的指数推算到今天), 并记下比例、指数期与出处.
pub(crate) fn apply_drift<'a>(
    benchmarks: &mut [Value],
    pick: impl Fn(&Value) -> Option<(&'a [IndexPoint], String)>,
) {
    for benchmark in benchmarks {
        let (Some(price), Some(date)) = (benchmark["price"].as_f64(), benchmark["date"].as_str())
        else {
            continue;
        };
        let Some((series, source)) = pick(benchmark) else {
            continue;
        };
        if let Some((ratio, index_date)) = index_ratio(series, date) {
            benchmark["current_price"] = json!((price * ratio * 100.0).round() / 100.0);
            benchmark["index_ratio"] = json!(ratio);
            benchmark["index_date"] = json!(index_date);
            benchmark["index_source"] = json!(source);
        }
    }
}

/// 指数序列 (master `price_factor.index_series`) 的最新值, 每条当一个基准: 标准品指标 + 当日指数值.
pub(crate) fn index_benchmarks(
    config: &[Value],
    history: &HashMap<String, Vec<IndexPoint>>,
) -> Vec<Value> {
    config
        .iter()
        .filter_map(|series| {
            let code = series["code"].as_str()?;
            let latest = history.get(code)?.last()?;
            let mut benchmark = json!({
                "name": series["name"].as_str()?,
                "props": series["props"].clone(),
                "price": latest.price,
                "date": latest.date,
                "source": format!("钢铁网国产炼焦煤价格指数 {code} (元/湿吨, 现金)"),
                "origin": "mcci",
                "index_code": code,
            });
            for key in ["coal_type", "form", "region"] {
                if let Some(text) = series[key].as_str() {
                    benchmark[key] = json!(text);
                }
            }
            Some(benchmark)
        })
        .collect()
}

/// 给旧报价挑推算用的指数: 同煤种、能推算这条报价 (有不晚于报价日的基点、也有更新的点) 的
/// 指数序列, 多档时取硫分最接近的; 没有就用 CCP —— 序列历史不全或停更时也就退回 CCP.
/// 指数自己生成的基准只看 CCP: 平时比 CCP 新、不动; 停更后 CCP 出了新一期才推算.
pub(crate) fn drift_series<'a>(
    benchmark: &Value,
    config: &[Value],
    history: &'a HashMap<String, Vec<IndexPoint>>,
    ccp: Option<&'a [IndexPoint]>,
) -> Option<(&'a [IndexPoint], String)> {
    let ccp = ccp.map(|points| (points, "CCP".to_string()));
    let Some(date) = benchmark["date"].as_str() else {
        return ccp;
    };
    if benchmark["origin"] == "mcci" {
        return ccp;
    }
    let sulfur = benchmark["props"]["S"].as_f64();
    let sulfur_gap = |series: &Value| match (sulfur, series["props"]["S"].as_f64()) {
        (Some(a), Some(b)) => (a - b).abs(),
        _ => f64::INFINITY,
    };
    let same_type = config
        .iter()
        .filter(|series| {
            benchmark["coal_type"].is_string() && series["coal_type"] == benchmark["coal_type"]
        })
        .filter_map(|series| {
            let code = series["code"].as_str()?;
            let points = history.get(code)?;
            index_ratio(points, date)?;
            Some((series, code, points))
        })
        .min_by(|(a, ..), (b, ..)| sulfur_gap(a).total_cmp(&sulfur_gap(b)));
    match same_type {
        Some((_, code, points)) => Some((points.as_slice(), format!("钢铁网 {code}"))),
        None => ccp,
    }
}

fn master_benchmarks(master: &Value) -> Vec<Value> {
    master["price_factor"]["benchmarks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut benchmark| {
            benchmark["origin"] = json!("master");
            benchmark
        })
        .collect()
}

/// 库表里每个名字最新的一条.
async fn latest_stored(pool: &PgPool) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (name)
               id, name, coal_type, form, region, props, price, quoted_on, source, created_by
        FROM price_benchmarks
        ORDER BY name, quoted_on DESC, created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(stored_row).collect()
}

fn stored_row(row: sqlx::postgres::PgRow) -> Result<Value, sqlx::Error> {
    let id: String = row.try_get("id")?;
    let quoted_on: NaiveDate = row.try_get("quoted_on")?;
    let mut benchmark = json!({
        "id": id,
        "name": row.try_get::<String, _>("name")?,
        "props": row.try_get::<Value, _>("props")?,
        "price": row.try_get::<f64, _>("price")?,
        "date": quoted_on.format("%Y-%m-%d").to_string(),
        "created_by": row.try_get::<String, _>("created_by")?,
        "origin": "stored",
    });
    for key in ["coal_type", "form", "region", "source"] {
        if let Some(text) = row.try_get::<Option<String>, _>(key)? {
            benchmark[key] = json!(text);
        }
    }
    Ok(benchmark)
}

/// 当前生效的基准报价: master ∪ 库表 (同名取最新) ∪ 钢铁网指数各序列的最新值; 旧报价按
/// 同煤种的指数序列推算到今天, 没有同煤种序列的按 CCP.
/// 库读失败只记日志, 退回 master 的那一层 —— 估价不该因为补录表坏了就停摆.
pub(crate) async fn current(state: &AppState, master: &Value) -> Vec<Value> {
    let (stored, history) = match &state.database {
        Some(pool) => {
            let stored = latest_stored(pool).await.unwrap_or_else(|error| {
                tracing::error!(%error, "读取基准报价失败, 只用 master 默认值");
                Vec::new()
            });
            let history = mcci::history(pool).await.unwrap_or_else(|error| {
                tracing::error!(%error, "读取钢铁网指数失败, 不用指数基准");
                HashMap::new()
            });
            (stored, history)
        }
        None => (Vec::new(), HashMap::new()),
    };
    let config = master["price_factor"]["index_series"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let indexed = index_benchmarks(&config, &history);
    let mut merged = merge(&[&master_benchmarks(master), &stored, &indexed]);
    let ccp = coal_index::get_spot_series().await;
    apply_drift(&mut merged, |benchmark| {
        drift_series(benchmark, &config, &history, ccp.as_deref())
    });
    merged
}

async fn master_value(state: &AppState) -> Value {
    serde_json::from_str(&merged_master_json(state).await).unwrap_or_default()
}

/// GET /api/benchmarks —— 当前生效的基准报价 (登录即可).
pub(crate) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }
    let master = master_value(&state).await;
    Json(json!({ "ok": true, "benchmarks": current(&state, &master).await })).into_response()
}

/// 校验并规范化一条新基准: 结构交给核心 (与估价同一套规则), 这里只补日期格式.
fn parse_benchmark(body: &[u8], master: &Value) -> Result<blend_kit::Benchmark, String> {
    let benchmark: blend_kit::Benchmark =
        serde_json::from_slice(body).map_err(|e| format!("基准报价格式不对: {e}"))?;
    NaiveDate::parse_from_str(&benchmark.date, "%Y-%m-%d")
        .map_err(|_| format!("报价日 {} 须为 YYYY-MM-DD", benchmark.date))?;
    if benchmark.current_price.is_some() {
        return Err("current_price 由服务端按指数推算, 不能手填".into());
    }
    let adjusts = serde_json::from_value(master["price_factor"]["adjusts"].clone())
        .map_err(|_| "煤库价格因子系数表缺失".to_string())?;
    let factor = blend_kit::PriceFactor {
        adjusts,
        benchmarks: vec![benchmark.clone()],
        ..Default::default()
    };
    blend_kit::price_factor::validate(&factor)?;
    Ok(benchmark)
}

/// POST /api/admin/benchmarks —— 补录一条基准报价 (管理员).
pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    let admin = match require_admin(&headers, &state) {
        Ok(admin) => admin.username.clone(),
        Err(response) => return *response,
    };
    if let Err(response) = require_json_content_type(&headers) {
        return *response;
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_BENCHMARK_BODY).await else {
        return bad_request("请求体读取失败或超出大小上限");
    };
    let master = master_value(&state).await;
    let benchmark = match parse_benchmark(&body, &master) {
        Ok(benchmark) => benchmark,
        Err(reason) => return bad_request(&reason),
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    let quoted_on = NaiveDate::parse_from_str(&benchmark.date, "%Y-%m-%d").expect("已校验");
    let id = Uuid::new_v4().to_string();
    let inserted = sqlx::query(
        r#"
        INSERT INTO price_benchmarks (id, name, coal_type, form, region, props, price, quoted_on, source, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(&id)
    .bind(benchmark.name.trim())
    .bind(&benchmark.coal_type)
    .bind(&benchmark.form)
    .bind(&benchmark.region)
    .bind(json!(benchmark.props))
    .bind(benchmark.price)
    .bind(quoted_on)
    .bind(&benchmark.source)
    .bind(&admin)
    .execute(pool)
    .await;
    match inserted {
        Ok(_) => {
            tracing::info!(name = %benchmark.name, price = benchmark.price, "已补录基准报价");
            Json(json!({ "ok": true, "id": id })).into_response()
        }
        Err(error) => database_error(error),
    }
}

/// DELETE /api/admin/benchmarks/{id} —— 删一条补录的基准报价 (录错了用). master 的删不了.
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
    // 非法 id 直接当作"不存在".
    if Uuid::parse_str(&id).is_err() {
        return not_found();
    }
    match sqlx::query("DELETE FROM price_benchmarks WHERE id = $1")
        .bind(&id)
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
        Json(json!({ "ok": false, "reason": "基准报价不存在" })),
    )
        .into_response()
}

/// 估价请求补全: 没给系数表就用煤库的, 没给基准就用当前生效的.
pub(crate) fn fill_price_request(request: &mut Value, master: &Value, benchmarks: &[Value]) {
    // 用煤库的系数表时, 里面带的 benchmarks 只是 master 默认值, 要换成当前生效的.
    let from_master = !request["price_factor"].is_object();
    if from_master {
        request["price_factor"] = master["price_factor"].clone();
        if let Some(factor) = request["price_factor"].as_object_mut() {
            for key in MASTER_ONLY_KEYS {
                factor.remove(key);
            }
        }
    }
    let missing = request["price_factor"]["benchmarks"]
        .as_array()
        .is_none_or(|list| list.is_empty());
    if (from_master || missing) && !benchmarks.is_empty() {
        request["price_factor"]["benchmarks"] = json!(benchmarks);
    }
}

/// POST /api/price —— 估价模块的独立入口: 只估价, 不求解 (登录即可).
/// 请求体 `{"coals": [...], "price_factor"?: {...}}`; 不给系数表或基准就用线上当前的.
pub(crate) async fn estimate(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_PRICE_BODY).await else {
        return bad_request("请求体读取失败或超出大小上限");
    };
    let Ok(mut input) = serde_json::from_slice::<Value>(&body) else {
        return bad_request("请求体必须是 JSON");
    };
    let master = master_value(&state).await;
    let benchmarks = current(&state, &master).await;
    fill_price_request(&mut input, &master, &benchmarks);
    let output = blend_kit::estimate_json(&input.to_string());
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        output,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(date: &str, price: f64) -> IndexPoint {
        IndexPoint {
            date: date.into(),
            price,
        }
    }

    #[test]
    fn test_merge_keeps_latest_per_name_and_stored_wins_ties() {
        let master = [
            json!({"name": "晋中", "coal_type": "焦煤", "date": "2026-10-10", "price": 2050}),
            json!({"name": "蒲县", "coal_type": "肥煤", "date": "2026-10-09", "price": 2145}),
        ];
        let stored = [
            json!({"name": "晋中", "coal_type": "焦煤", "date": "2026-10-10", "price": 2040}),
            json!({"name": "蒲县", "coal_type": "肥煤", "date": "2026-10-01", "price": 1900}),
        ];
        let merged = merge(&[&master, &stored]);
        assert_eq!(merged.len(), 2);
        // 同日以库表为准; 库表更旧的不覆盖. 按煤种排序: 焦煤 < 肥煤 (按字节).
        let price =
            |name: &str| merged.iter().find(|b| b["name"] == name).unwrap()["price"].clone();
        assert_eq!(price("晋中"), 2040);
        assert_eq!(price("蒲县"), 2145);
    }

    #[test]
    fn test_index_drift_only_after_quote_date() {
        let series = [point("2026-09-24", 1975.0), point("2026-10-08", 1907.0)];
        // 报价在 9/30, 指数 10/08 比 9/24 那期跌了: 推算到今天.
        let (ratio, date) = index_ratio(&series, "2026-09-30").unwrap();
        assert!((ratio - 1907.0 / 1975.0).abs() < 1e-12);
        assert_eq!(date, "2026-10-08");
        // 报价日不早于最新一期, 或早于整条序列: 不推算.
        assert!(index_ratio(&series, "2026-10-10").is_none());
        assert!(index_ratio(&series, "2026-10-08").is_none());
        assert!(index_ratio(&series, "2026-01-01").is_none());

        let mut benchmarks = vec![
            json!({"name": "旧", "date": "2026-09-30", "price": 2000.0}),
            json!({"name": "新", "date": "2026-10-10", "price": 2000.0}),
        ];
        apply_drift(&mut benchmarks, |_| Some((&series[..], "CCP".to_string())));
        assert_eq!(benchmarks[0]["current_price"], 1931.14);
        assert_eq!(benchmarks[0]["index_date"], "2026-10-08");
        assert_eq!(benchmarks[0]["index_source"], "CCP");
        assert!(benchmarks[1].get("current_price").is_none());
    }

    fn index_config() -> Vec<Value> {
        vec![
            json!({"code": "LJM_DLZJM", "type_name": "低硫主焦煤", "name": "钢铁网低硫主焦煤指数",
                   "coal_type": "焦煤", "form": "精煤", "region": "山西",
                   "props": {"A": 9.5, "S": 0.6, "V": 25, "G": 85, "M": 10}}),
            json!({"code": "LJM_GLZJM", "type_name": "高硫主焦煤", "name": "钢铁网高硫主焦煤指数",
                   "coal_type": "焦煤", "form": "精煤", "region": "山西",
                   "props": {"A": 10.5, "S": 2.5, "V": 25, "G": 80, "M": 10}}),
            json!({"code": "LJM_QM", "type_name": "气煤", "name": "钢铁网气煤指数",
                   "coal_type": "气煤", "props": {"A": 9, "S": 0.6, "V": 39, "G": 75, "M": 8}}),
        ]
    }

    fn index_history() -> HashMap<String, Vec<IndexPoint>> {
        HashMap::from([
            (
                "LJM_DLZJM".to_string(),
                vec![point("2026-10-15", 2294.0), point("2026-10-26", 2321.0)],
            ),
            (
                "LJM_GLZJM".to_string(),
                vec![point("2026-10-15", 2118.6), point("2026-10-26", 2107.4)],
            ),
        ])
    }

    #[test]
    fn test_index_benchmarks_take_latest_value_with_standard_spec() {
        let indexed = index_benchmarks(&index_config(), &index_history());
        // 气煤没有数据: 不出基准.
        assert_eq!(indexed.len(), 2);
        let low = indexed
            .iter()
            .find(|b| b["name"] == "钢铁网低硫主焦煤指数")
            .unwrap();
        assert_eq!(low["price"], 2321.0);
        assert_eq!(low["date"], "2026-10-26");
        assert_eq!(low["coal_type"], "焦煤");
        assert_eq!(low["props"]["S"], 0.6);
        assert_eq!(low["origin"], "mcci");
        assert_eq!(low["index_code"], "LJM_DLZJM");
        // 能直接当核心的基准用.
        let parsed: blend_kit::Benchmark = serde_json::from_value(low.clone()).unwrap();
        assert_eq!(parsed.price, 2321.0);
    }

    #[test]
    fn test_drift_series_prefers_same_coal_type_nearest_sulfur() {
        let config = index_config();
        let history = index_history();
        let ccp = [
            point("2026-09-24", 1975.0),
            point("2026-10-08", 1907.0),
            point("2026-10-23", 1890.0),
        ];
        let pick = |b: Value| drift_series(&b, &config, &history, Some(&ccp)).map(|(_, l)| l);
        // 焦煤 S2.9 → 高硫档; S0.5 → 低硫档.
        assert_eq!(
            pick(json!({"coal_type": "焦煤", "date": "2026-10-20", "props": {"S": 2.9}}))
                .as_deref(),
            Some("钢铁网 LJM_GLZJM")
        );
        assert_eq!(
            pick(json!({"coal_type": "焦煤", "date": "2026-10-20", "props": {"S": 0.5}}))
                .as_deref(),
            Some("钢铁网 LJM_DLZJM")
        );
        // 配了序列但没数据 (气煤) 与没配序列的煤种 (贫瘦煤): 退回 CCP.
        assert_eq!(
            pick(json!({"coal_type": "气煤", "date": "2026-10-20", "props": {"S": 0.6}}))
                .as_deref(),
            Some("CCP")
        );
        assert_eq!(
            pick(json!({"coal_type": "贫瘦煤", "date": "2026-10-20", "props": {"S": 0.4}}))
                .as_deref(),
            Some("CCP")
        );
        // 报价早于序列的第一天 (历史没回填全): 同煤种序列推算不了, 退回 CCP 而不是不推算.
        assert_eq!(
            pick(json!({"coal_type": "焦煤", "date": "2026-10-01", "props": {"S": 2.9}}))
                .as_deref(),
            Some("CCP")
        );
        // 没有 CCP 也没有同煤种序列: 不推算.
        assert!(drift_series(
            &json!({"coal_type": "贫瘦煤", "date": "2026-10-20", "props": {}}),
            &config,
            &history,
            None
        )
        .is_none());
    }

    #[test]
    fn test_index_benchmark_drifts_by_ccp_only_once_stale() {
        let config = index_config();
        let history = index_history();
        let ccp = [point("2026-09-24", 1975.0), point("2026-10-23", 1890.0)];
        let mut benchmarks = vec![
            // 指数停在 10/26, 比 CCP 新: 不推算.
            json!({"name": "新鲜", "coal_type": "焦煤", "origin": "mcci", "props": {"S": 0.6},
                   "date": "2026-10-26", "price": 2321.0}),
            // 指数卡在 10/01 (签名失效之类), CCP 之后又更新了: 按 CCP 推算, 不让旧价冒充当前.
            json!({"name": "停更", "coal_type": "焦煤", "origin": "mcci", "props": {"S": 0.6},
                   "date": "2026-10-01", "price": 2300.0}),
        ];
        apply_drift(&mut benchmarks, |b| {
            drift_series(b, &config, &history, Some(&ccp))
        });
        assert!(benchmarks[0].get("current_price").is_none());
        assert_eq!(benchmarks[1]["index_source"], "CCP");
        assert_eq!(benchmarks[1]["current_price"], 2201.01);
    }

    #[test]
    fn test_old_quote_drifts_by_its_own_coal_type_series() {
        let config = index_config();
        let history = index_history();
        let mut benchmarks = vec![json!({"name": "孝义高硫", "coal_type": "焦煤",
            "props": {"S": 2.9}, "date": "2026-10-15", "price": 2039.0})];
        apply_drift(&mut benchmarks, |b| {
            drift_series(b, &config, &history, None)
        });
        // 高硫档 10/15 → 10/26: 2107.4 / 2118.6.
        assert_eq!(benchmarks[0]["current_price"], 2028.22);
        assert_eq!(benchmarks[0]["index_source"], "钢铁网 LJM_GLZJM");
        assert_eq!(benchmarks[0]["index_date"], "2026-10-26");
    }

    #[test]
    fn test_parse_benchmark_validates_with_core_rules() {
        let master = json!({"price_factor": {"adjusts": [
            {"indicator": "S", "direction": "Upper", "base": 1.3, "premium_rate": 75, "discount_rate": 125}
        ]}});
        let good =
            r#"{"name":"x","coal_type":"焦煤","props":{"S":1.3},"price":2000,"date":"2026-10-16"}"#;
        assert!(parse_benchmark(good.as_bytes(), &master).is_ok());
        let bad_date = br#"{"name":"x","props":{"S":1.3},"price":2000,"date":"10/16"}"#;
        assert!(parse_benchmark(bad_date, &master)
            .unwrap_err()
            .contains("YYYY-MM-DD"));
        let bad_price = br#"{"name":"x","props":{"S":1.3},"price":-5,"date":"2026-10-16"}"#;
        assert!(parse_benchmark(bad_price, &master).is_err());
        let forged =
            br#"{"name":"x","props":{"S":1.3},"price":2000,"date":"2026-10-16","current_price":1}"#;
        assert!(parse_benchmark(forged, &master).is_err());
    }

    #[test]
    fn test_fill_price_request_uses_master_and_current_benchmarks() {
        let master = json!({"price_factor": {"description": "说明", "adjusts": [],
            "index_series": [{"code": "LJM_QM"}], "index_series_note": "说明"}});
        let benchmarks = [json!({"name": "晋中", "price": 2050})];
        let mut request = json!({"coals": []});
        fill_price_request(&mut request, &master, &benchmarks);
        assert!(request["price_factor"].get("description").is_none());
        assert!(request["price_factor"].get("index_series").is_none());
        assert!(request["price_factor"].get("index_series_note").is_none());
        assert_eq!(request["price_factor"]["benchmarks"][0]["name"], "晋中");

        // master 自己带着默认基准时, 也要换成当前生效的 (补录、指数、推算后的).
        let master_with_defaults = json!({"price_factor": {"adjusts": [],
            "benchmarks": [{"name": "master 默认", "price": 2000}]}});
        let mut request = json!({"coals": []});
        fill_price_request(&mut request, &master_with_defaults, &benchmarks);
        assert_eq!(request["price_factor"]["benchmarks"][0]["name"], "晋中");

        // 调用方自带基准就不动.
        let mut own =
            json!({"coals": [], "price_factor": {"adjusts": [], "benchmarks": [{"name": "自带"}]}});
        fill_price_request(&mut own, &master, &benchmarks);
        assert_eq!(own["price_factor"]["benchmarks"][0]["name"], "自带");
    }
}
