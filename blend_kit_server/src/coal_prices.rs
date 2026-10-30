//! 共享煤库的每日估价快照与历史补算; 用户修改的化验值只由 /api/price 现算.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{
    bad_request, benchmarks, database_error, is_authenticated, mcci, merged_master_json_from_pool,
    require_database, unauthorized, AppState,
};

#[derive(Debug, Clone, sqlx::FromRow)]
struct PriceRow {
    name: String,
    price: f64,
    points: f64,
    benchmark: Option<String>,
    benchmark_price: Option<f64>,
    #[sqlx(json)]
    missing: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct StoredRow {
    day: NaiveDate,
    computed_at: DateTime<Utc>,
    #[sqlx(flatten)]
    price: PriceRow,
}

struct DaySnapshot {
    day: NaiveDate,
    computed_at: DateTime<Utc>,
    rows: Vec<PriceRow>,
}

/// /api/coal-prices 的单煤估价; 基准的生效价仅存库, 不向卡片暴露.
#[derive(Serialize)]
pub struct CoalPriceEntry {
    pub name: String,
    pub price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_price: Option<f64>,
    pub points: f64,
    pub benchmark: Option<String>,
    pub missing: Vec<String>,
}

#[derive(Serialize)]
pub struct CoalPriceSnapshot {
    pub ok: bool,
    pub day: Option<NaiveDate>,
    pub prev_day: Option<NaiveDate>,
    pub computed_at: DateTime<Utc>,
    pub prices: Vec<CoalPriceEntry>,
}

fn beijing_day(now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&mcci::beijing()).date_naive()
}

fn snapshot_coal(coal: &Value) -> Result<Value, String> {
    let props = coal["props"].as_object().ok_or("煤的指标须为对象")?;
    let props: serde_json::Map<String, Value> = props
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(json!({
        "name": coal["name"], "props": props, "fob": 0, "frt": 0,
        "coal_type": coal["coal_type"], "form": coal["form"], "region": coal["region"]
    }))
}

fn snapshot_rows(master: &Value, benchmarks: &[Value]) -> Result<Vec<PriceRow>, String> {
    if !master["price_factor"]["adjusts"].is_array() {
        return Err("煤库价格因子系数表缺失或格式不对".into());
    }
    let coals = master["coals"].as_array().ok_or("煤库缺少煤种列表")?;
    let inputs = coals
        .iter()
        .map(snapshot_coal)
        .collect::<Result<Vec<_>, _>>()?;
    let mut request = json!({"coals": inputs});
    benchmarks::fill_price_request(&mut request, master, benchmarks);
    let result: blend_kit::PriceResult =
        serde_json::from_str(&blend_kit::estimate_json(&request.to_string()))
            .map_err(|error| error.to_string())?;
    if !result.ok {
        return Err(result.reason.unwrap_or_else(|| "估价失败".into()));
    }
    if result.coal_values.len() != coals.len() {
        return Err("估价结果未覆盖全部煤种, 保留旧快照".into());
    }
    let effective = request["price_factor"]["benchmarks"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    Ok(result
        .coal_values
        .into_iter()
        .map(|value| price_row(value, effective))
        .collect())
}

fn price_row(value: blend_kit::CoalValue, benchmarks: &[Value]) -> PriceRow {
    let benchmark_price = benchmarks
        .iter()
        .find(|benchmark| benchmark["name"].as_str() == value.benchmark.as_deref())
        .and_then(|benchmark| {
            benchmark["current_price"]
                .as_f64()
                .or_else(|| benchmark["price"].as_f64())
        });
    PriceRow {
        name: value.name,
        price: value.fair,
        points: value.points,
        benchmark: value.benchmark,
        benchmark_price,
        missing: value.missing,
    }
}

fn compose(latest: Option<DaySnapshot>, prev: Option<DaySnapshot>) -> Value {
    let Some(latest) = latest else {
        return json!({"ok": true, "day": null, "prices": []});
    };
    let prev_day = prev.as_ref().map(|snapshot| snapshot.day);
    let previous: HashMap<_, _> = prev
        .into_iter()
        .flat_map(|snapshot| snapshot.rows)
        .map(|row| (row.name, row.price))
        .collect();
    let prices = latest
        .rows
        .into_iter()
        .map(|row| CoalPriceEntry {
            prev_price: previous.get(&row.name).copied(),
            name: row.name,
            price: row.price,
            points: row.points,
            benchmark: row.benchmark,
            missing: row.missing,
        })
        .collect();
    json!(CoalPriceSnapshot {
        ok: true,
        day: Some(latest.day),
        prev_day,
        computed_at: latest.computed_at,
        prices,
    })
}

/// 补算范围: 最早一条能定价的输入 (指数各序列首日、master 与库表报价日) 与 400 天前取晚的,
/// 到昨天为止 —— 今天归实时快照管. 也不越过指数最新一期: 某天的指数当晚才入库, 早于它补
/// 只能用前一天的指数, 而补下去就定格了. 没有输入、或起点晚于终点: 不补.
fn backfill_range(
    today: NaiveDate,
    earliest: Option<NaiveDate>,
    latest_index: Option<NaiveDate>,
) -> Option<(NaiveDate, NaiveDate)> {
    let start = earliest?.max(today - Days::new(mcci::HISTORY_DAYS));
    let yesterday = today.pred_opt()?;
    let end = latest_index.map_or(yesterday, |latest| latest.min(yesterday));
    (start <= end).then_some((start, end))
}

/// 各指数序列最新一期中最新的那天; 一条停更的序列不拖住其余的.
fn latest_index(inputs: &benchmarks::BenchmarkInputs) -> Option<NaiveDate> {
    inputs
        .history
        .values()
        .filter_map(|points| points.last())
        .filter_map(|point| point.date.parse::<NaiveDate>().ok())
        .max()
}

/// CCP 只用来推算涨跌, 单独定不出价, 不算输入.
fn earliest_input(master: &Value, inputs: &benchmarks::BenchmarkInputs) -> Option<NaiveDate> {
    let quotes = master["price_factor"]["benchmarks"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(&inputs.stored)
        .filter_map(|benchmark| benchmark["date"].as_str());
    let index = inputs
        .history
        .values()
        .filter_map(|points| points.first())
        .map(|point| point.date.as_str());
    quotes
        .chain(index)
        .filter_map(|date| date.parse::<NaiveDate>().ok())
        .min()
}

/// 范围内逐日列出还缺的煤; 一种都不缺的天不出现.
fn missing_by_day(
    (start, end): (NaiveDate, NaiveDate),
    names: &[String],
    existing: &HashSet<(NaiveDate, String)>,
) -> Vec<(NaiveDate, BTreeSet<String>)> {
    start
        .iter_days()
        .take_while(|day| *day <= end)
        .filter_map(|day| {
            let missing: BTreeSet<String> = names
                .iter()
                .filter(|name| !existing.contains(&(day, (*name).clone())))
                .cloned()
                .collect();
            (!missing.is_empty()).then_some((day, missing))
        })
        .collect()
}

type BackfillRows = (Vec<(NaiveDate, PriceRow)>, Vec<(NaiveDate, String)>);

/// 逐日按 as_of(那天) 估价, 只留缺的煤; 估不出价的天 (如一个基准都没有) 跳过并记下原因.
fn backfill_rows(
    master: &Value,
    inputs: &benchmarks::BenchmarkInputs,
    plan: &[(NaiveDate, BTreeSet<String>)],
) -> BackfillRows {
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for (day, missing) in plan {
        let benchmarks = benchmarks::as_of(master, inputs, *day);
        // 不能把空基准交给 snapshot_rows: fill_price_request 在基准为空时退回 master 自带的,
        // 那里有晚于这天的报价 —— 等于偷看未来.
        if benchmarks.is_empty() {
            skipped.push((*day, "这天还没有任何基准".into()));
            continue;
        }
        match snapshot_rows(master, &benchmarks) {
            Ok(priced) => rows.extend(
                priced
                    .into_iter()
                    .filter(|row| missing.contains(&row.name))
                    .map(|row| (*day, row)),
            ),
            Err(reason) => skipped.push((*day, reason)),
        }
    }
    (rows, skipped)
}

/// 删写在同一个事务内, 插入失败则原快照完整保留. 实时行, source 取默认 live.
async fn write_snapshot(
    pool: &PgPool,
    day: NaiveDate,
    rows: &[PriceRow],
) -> Result<(), sqlx::Error> {
    let names: Vec<_> = rows.iter().map(|row| row.name.as_str()).collect();
    let prices: Vec<_> = rows.iter().map(|row| row.price).collect();
    let points: Vec<_> = rows.iter().map(|row| row.points).collect();
    let benchmarks: Vec<_> = rows.iter().map(|row| row.benchmark.as_deref()).collect();
    let benchmark_prices: Vec<_> = rows.iter().map(|row| row.benchmark_price).collect();
    let missing: Vec<_> = rows.iter().map(|row| json!(row.missing)).collect();
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM coal_price_daily WHERE day = $1")
        .bind(day)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        r#"
        INSERT INTO coal_price_daily (day, name, price, points, benchmark, benchmark_price, missing)
        SELECT $1, name, price, points, benchmark, benchmark_price, missing
        FROM UNNEST($2::text[], $3::float8[], $4::float8[], $5::text[], $6::float8[], $7::jsonb[])
            AS t(name, price, points, benchmark, benchmark_price, missing)
        "#,
    )
    .bind(day)
    .bind(&names)
    .bind(&prices)
    .bind(&points)
    .bind(&benchmarks)
    .bind(&benchmark_prices)
    .bind(&missing)
    .execute(&mut *transaction)
    .await?;
    // 读库或锁等待跨过零点时回滚, 不能再改昨天.
    if day != beijing_day(Utc::now()) {
        return Err(sqlx::Error::Protocol("估价写入时已跨日, 保留旧快照".into()));
    }
    transaction.commit().await
}

async fn refresh(pool: &PgPool) -> Result<(), String> {
    let day = beijing_day(Utc::now());
    let master: Value = serde_json::from_str(&merged_master_json_from_pool(Some(pool)).await?)
        .map_err(|error| error.to_string())?;
    let (benchmarks, error) = benchmarks::current_from_pool(Some(pool), &master).await;
    if let Some(error) = error {
        return Err(error);
    }
    let rows = snapshot_rows(&master, &benchmarks)?;
    write_snapshot(pool, day, &rows)
        .await
        .map_err(|error| error.to_string())
}

/// 补算只接受完整输入, 空 CCP 也视为不可用; 交互估价仍可逐层降级.
fn require_backfill_inputs(
    master: &Value,
    inputs: benchmarks::BenchmarkInputs,
    error: Option<String>,
) -> Result<benchmarks::BenchmarkInputs, String> {
    if let Some(error) = error {
        return Err(error);
    }
    if inputs.ccp.as_ref().is_none_or(Vec::is_empty) {
        return Err("CCP 指数取不到, 推算缺一层, 不补算".into());
    }
    // 首次启动时指数逐条入库, 查询成功也可能还缺序列; 不能把缺料的估价永久定格.
    for code in master["price_factor"]["index_series"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|series| series["code"].as_str())
    {
        if inputs.history.get(code).is_none_or(Vec::is_empty) {
            return Err(format!("钢铁网指数序列 {code} 尚无历史, 不补算"));
        }
    }
    Ok(inputs)
}

pub(crate) struct BackfillReport {
    pub days: usize,
    pub rows: u64,
    pub skipped: Vec<(NaiveDate, String)>,
}

async fn existing_keys(
    pool: &PgPool,
    (start, end): (NaiveDate, NaiveDate),
) -> Result<HashSet<(NaiveDate, String)>, sqlx::Error> {
    let rows: Vec<(NaiveDate, String)> =
        sqlx::query_as("SELECT day, name FROM coal_price_daily WHERE day BETWEEN $1 AND $2")
            .bind(start)
            .bind(end)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// 补算的行只填空: 已有的 (实时定格的、之前补过的) 一律不动. 每批自身幂等, 中途失败下次接着补.
async fn insert_backfill(
    pool: &PgPool,
    rows: &[(NaiveDate, PriceRow)],
) -> Result<u64, sqlx::Error> {
    let mut inserted = 0;
    for chunk in rows.chunks(5000) {
        let days: Vec<_> = chunk.iter().map(|(day, _)| *day).collect();
        let names: Vec<_> = chunk.iter().map(|(_, row)| row.name.as_str()).collect();
        let prices: Vec<_> = chunk.iter().map(|(_, row)| row.price).collect();
        let points: Vec<_> = chunk.iter().map(|(_, row)| row.points).collect();
        let benchmarks: Vec<_> = chunk
            .iter()
            .map(|(_, row)| row.benchmark.as_deref())
            .collect();
        let benchmark_prices: Vec<_> = chunk.iter().map(|(_, row)| row.benchmark_price).collect();
        let missing: Vec<_> = chunk.iter().map(|(_, row)| json!(row.missing)).collect();
        inserted += sqlx::query(
            r#"
            INSERT INTO coal_price_daily
                (day, name, price, points, benchmark, benchmark_price, missing, source)
            SELECT day, name, price, points, benchmark, benchmark_price, missing, 'backfill'
            FROM UNNEST($1::date[], $2::text[], $3::float8[], $4::float8[], $5::text[],
                        $6::float8[], $7::jsonb[])
                AS t(day, name, price, points, benchmark, benchmark_price, missing)
            ON CONFLICT (day, name) DO NOTHING
            "#,
        )
        .bind(&days)
        .bind(&names)
        .bind(&prices)
        .bind(&points)
        .bind(&benchmarks)
        .bind(&benchmark_prices)
        .bind(&missing)
        .execute(pool)
        .await?
        .rows_affected();
    }
    Ok(inserted)
}

/// 补齐 today 以前缺的 (日期, 煤). 输入读不全 (库表/指数读失败、CCP 取不到) 整轮不写:
/// 补下去就定格了, 不能拿缺料的结果填.
pub(crate) async fn backfill(pool: &PgPool, today: NaiveDate) -> Result<BackfillReport, String> {
    let master: Value = serde_json::from_str(&merged_master_json_from_pool(Some(pool)).await?)
        .map_err(|error| error.to_string())?;
    let (inputs, error) = benchmarks::load_inputs(Some(pool)).await;
    let inputs = require_backfill_inputs(&master, inputs, error)?;
    let range = backfill_range(
        today,
        earliest_input(&master, &inputs),
        latest_index(&inputs),
    );
    let Some(range) = range else {
        return Ok(BackfillReport {
            days: 0,
            rows: 0,
            skipped: Vec::new(),
        });
    };
    let names: Vec<String> = master["coals"]
        .as_array()
        .ok_or("煤库缺少煤种列表")?
        .iter()
        .filter_map(|coal| coal["name"].as_str().map(str::to_string))
        .collect();
    let existing = existing_keys(pool, range)
        .await
        .map_err(|error| error.to_string())?;
    let plan = missing_by_day(range, &names, &existing);
    let days = plan.len();
    // 逐日估价是同步的 CPU 活, 不占异步线程.
    let (rows, skipped) =
        tokio::task::spawn_blocking(move || backfill_rows(&master, &inputs, &plan))
            .await
            .map_err(|error| error.to_string())?;
    let rows = insert_backfill(pool, &rows)
        .await
        .map_err(|error| error.to_string())?;
    Ok(BackfillReport {
        days,
        rows,
        skipped,
    })
}

/// interval 首次立即触发; 不受 MCCI_FETCH 开关影响, 每轮 panic 由子任务隔离.
/// 每轮先算今天 (卡片价优先), 再看今天补过历史没有: 启动时一次, 之后每过零点一次; 补失败下小时重试.
pub(crate) fn spawn_hourly(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        let mut backfilled_on: Option<NaiveDate> = None;
        loop {
            ticker.tick().await;
            let round_pool = pool.clone();
            match tokio::spawn(async move { refresh(&round_pool).await }).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!(%error, "每日估价快照更新失败, 保留旧数据"),
                Err(error) => tracing::error!(%error, "每日估价快照这一轮异常退出, 下个小时再来"),
            }
            let today = beijing_day(Utc::now());
            if backfilled_on == Some(today) {
                continue;
            }
            let round_pool = pool.clone();
            match tokio::spawn(async move { backfill(&round_pool, today).await }).await {
                Ok(Ok(report)) => {
                    let first_skipped = report
                        .skipped
                        .first()
                        .map(|(day, reason)| format!("{day}: {reason}"));
                    tracing::info!(
                        days = report.days,
                        rows = report.rows,
                        skipped = report.skipped.len(),
                        first_skipped = first_skipped.as_deref().unwrap_or("-"),
                        "历史估价补算完成"
                    );
                    backfilled_on = Some(today);
                }
                Ok(Err(error)) => tracing::error!(%error, "历史估价补算失败, 下个小时再试"),
                Err(error) => tracing::error!(%error, "历史估价补算这一轮异常退出, 下个小时再试"),
            }
        }
    });
}

async fn read_snapshots(pool: &PgPool) -> Result<Value, sqlx::Error> {
    let rows = sqlx::query_as::<_, StoredRow>(
        r#"SELECT day, name, price, points, benchmark, benchmark_price, missing,
                  MAX(computed_at) OVER (PARTITION BY day) AS computed_at
           FROM coal_price_daily
           WHERE day IN (SELECT DISTINCT day FROM coal_price_daily ORDER BY day DESC LIMIT 2)
           ORDER BY day DESC, name"#,
    )
    .fetch_all(pool)
    .await?;
    let mut days = BTreeMap::new();
    for row in rows {
        days.entry(row.day)
            .or_insert_with(|| DaySnapshot {
                day: row.day,
                computed_at: row.computed_at,
                rows: Vec::new(),
            })
            .rows
            .push(row.price);
    }
    let mut snapshots = days.into_values().rev();
    Ok(compose(snapshots.next(), snapshots.next()))
}

pub(crate) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match read_snapshots(pool).await {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(error) => database_error(error),
    }
}

/// /api/coal-prices/history 的一天. source: live = 实时快照定格, backfill = 事后补算.
#[derive(Serialize, sqlx::FromRow)]
pub struct CoalPricePoint {
    pub day: NaiveDate,
    pub price: f64,
    pub benchmark: Option<String>,
    pub source: String,
}

/// 一种煤的历史估价, 按日升序; 煤库里没有这种煤时为空.
#[derive(Serialize)]
pub struct CoalPriceHistory {
    pub ok: bool,
    pub name: String,
    pub points: Vec<CoalPricePoint>,
}

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    name: Option<String>,
}

async fn read_history(pool: &PgPool, name: &str) -> Result<Vec<CoalPricePoint>, sqlx::Error> {
    sqlx::query_as(
        "SELECT day, price, benchmark, source FROM coal_price_daily WHERE name = $1 ORDER BY day",
    )
    .bind(name)
    .fetch_all(pool)
    .await
}

pub(crate) async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }
    let Some(name) = query.name.filter(|name| !name.is_empty()) else {
        return bad_request("缺少煤名 name");
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match read_history(pool, &name).await {
        Ok(points) => Json(json!(CoalPriceHistory {
            ok: true,
            name,
            points
        }))
        .into_response(),
        Err(error) => database_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coal_index::IndexPoint;
    use serde_json::json;

    fn at(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    fn fixture() -> (Value, Vec<Value>) {
        let master = json!({
            "coals": [
                {"name": "主力", "status": "verified", "coal_type": "主焦煤", "form": "精煤",
                 "region": "山西", "props": {"S": 1.0, "Y": 20.0}, "fob": 9999, "frt": 888},
                {"name": "停用", "status": "archived", "props": {"S": 1.0, "Y": 21.0}},
                {"name": "缺项", "status": "incomplete", "props": {"S": 1.0, "Y": null}}
            ],
            "price_factor": {
                "adjusts": [{"indicator": "Y", "direction": "Lower", "base": 20.0,
                             "premium_rate": 10.0, "discount_rate": 10.0}],
                "benchmarks": [{"name": "旧基准", "props": {"Y": 20.0}, "price": 1000,
                                "date": "2026-10-01"}],
                "description": "测试系数", "index_series": [], "index_series_note": "测试"
            }
        });
        let benchmarks = vec![json!({
            "name": "当前基准", "coal_type": "主焦煤", "form": "精煤", "region": "山西",
            "props": {"S": 1.0, "Y": 20.0}, "price": 2000.0,
            "current_price": 2100.0, "date": "2026-10-27"
        })];
        (master, benchmarks)
    }

    #[test]
    fn test_beijing_day_at_midnight() {
        assert_eq!(
            beijing_day(at("2026-10-28T15:59:00Z")).to_string(),
            "2026-10-28"
        );
        assert_eq!(
            beijing_day(at("2026-10-28T16:00:00Z")).to_string(),
            "2026-10-29"
        );
    }

    #[test]
    fn test_snapshot_rows_include_archived_and_missing_indicators() {
        let (master, benchmarks) = fixture();
        let rows = snapshot_rows(&master, &benchmarks).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "主力");
        assert_eq!(rows[0].price, 2100.0);
        assert_eq!(rows[0].points, 1000.0);
        assert_eq!(rows[0].benchmark.as_deref(), Some("当前基准"));
        assert_eq!(rows[0].benchmark_price, Some(2100.0));
        assert_eq!(rows[1].name, "停用");
        assert_eq!(rows[1].price, 2121.0);
        assert_eq!(rows[2].missing, ["Y"]);
    }

    #[test]
    fn test_snapshot_rows_use_quoted_price_without_current_price() {
        let (master, mut benchmarks) = fixture();
        benchmarks[0]
            .as_object_mut()
            .unwrap()
            .remove("current_price");
        let rows = snapshot_rows(&master, &benchmarks).unwrap();
        assert_eq!(rows[0].price, 2000.0);
        assert_eq!(rows[0].benchmark_price, Some(2000.0));
    }

    #[test]
    fn test_snapshot_rows_market_price_has_no_benchmark() {
        let (mut master, _) = fixture();
        master["price_factor"] = json!({"adjusts": [], "market_price": 1800.0});
        let rows = snapshot_rows(&master, &[]).unwrap();
        assert_eq!(rows[0].price, 1800.0);
        assert!(rows[0].benchmark.is_none());
        assert!(rows[0].benchmark_price.is_none());
    }

    #[test]
    fn test_snapshot_rows_reject_invalid_factors_and_unpriced_results() {
        let (mut master, benchmarks) = fixture();
        for factor in [
            json!([]),
            Value::Null,
            json!({"adjusts": [{"indicator": "不存在"}]}),
        ] {
            master["price_factor"] = factor;
            assert!(snapshot_rows(&master, &benchmarks).is_err());
        }
        master["price_factor"] = json!({"adjusts": []});
        assert!(snapshot_rows(&master, &[]).is_err());
    }

    #[test]
    fn test_snapshot_rows_require_explicit_adjusts() {
        let (mut master, benchmarks) = fixture();
        master["price_factor"]
            .as_object_mut()
            .unwrap()
            .remove("adjusts");
        assert!(snapshot_rows(&master, &benchmarks).is_err());
    }

    fn snapshot(day: &str, names: &[&str], price: f64) -> DaySnapshot {
        DaySnapshot {
            day: day.parse().unwrap(),
            computed_at: at(&format!("{day}T09:00:03Z")),
            rows: names
                .iter()
                .map(|name| PriceRow {
                    name: (*name).into(),
                    price,
                    points: 1000.0,
                    benchmark: Some("当前基准".into()),
                    benchmark_price: Some(price),
                    missing: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn test_compose_matches_previous_prices_by_name() {
        let result = compose(
            Some(snapshot("2026-10-28", &["乙", "新增", "甲"], 2100.0)),
            Some(snapshot("2026-10-26", &["甲", "乙"], 2000.0)),
        );
        assert_eq!(result["day"], "2026-10-28");
        assert_eq!(result["prev_day"], "2026-10-26");
        assert_eq!(result["computed_at"], "2026-10-28T09:00:03Z");
        assert_eq!(result["prices"][0]["prev_price"], 2000.0);
        assert!(result["prices"][1].get("prev_price").is_none());
        assert_eq!(result["prices"][2]["prev_price"], 2000.0);
        assert!(result["prices"][0].get("benchmark_price").is_none());
    }

    #[test]
    fn test_compose_without_previous_or_any_snapshot() {
        let result = compose(Some(snapshot("2026-10-28", &["甲"], 2100.0)), None);
        assert_eq!(result["ok"], true);
        assert!(result["prev_day"].is_null());
        assert!(result["prices"][0].get("prev_price").is_none());
        assert_eq!(
            compose(None, None),
            json!({"ok": true, "day": null, "prices": []})
        );
    }

    fn d(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn test_backfill_range_caps_at_history_window_and_stops_before_today() {
        let today = d("2026-10-30");
        assert_eq!(
            backfill_range(today, Some(d("2025-10-27")), None),
            Some((d("2025-10-27"), d("2026-10-29")))
        );
        // 输入比 400 天还早: 起点截到 400 天前.
        assert_eq!(
            backfill_range(today, Some(d("2020-01-01")), None),
            Some((d("2025-09-25"), d("2026-10-29")))
        );
        // 没有输入, 或最早的输入就是今天: 不补.
        assert_eq!(backfill_range(today, None, None), None);
        assert_eq!(backfill_range(today, Some(today), None), None);
    }

    #[test]
    fn test_backfill_range_waits_for_that_days_index() {
        let today = d("2026-10-30");
        // 停机一整天 (10-29) 后上午重启: 10-29 的指数当晚 17:30 才发布, 还没入库;
        // 这时补 10-29 只能用 10-28 的指数, 一写就定格 —— 终点停在指数最新一期, 等它到了再补.
        assert_eq!(
            backfill_range(today, Some(d("2025-10-27")), Some(d("2026-10-28"))),
            Some((d("2025-10-27"), d("2026-10-28")))
        );
        // 指数比昨天还新 (今天的已入库): 照旧到昨天为止.
        assert_eq!(
            backfill_range(today, Some(d("2025-10-27")), Some(today)),
            Some((d("2025-10-27"), d("2026-10-29")))
        );
    }

    #[test]
    fn test_latest_index_takes_the_newest_point_across_series() {
        let point = |date: &str| IndexPoint {
            date: date.into(),
            price: 1.0,
        };
        let inputs = benchmarks::BenchmarkInputs {
            stored: vec![],
            // 一条停更的序列不拖住其余的: 取各序列最新一期中最新的.
            history: HashMap::from([
                (
                    "LJM_QM".to_string(),
                    vec![point("2026-10-01"), point("2026-10-20")],
                ),
                (
                    "LJM_FM".to_string(),
                    vec![point("2026-10-01"), point("2026-10-28")],
                ),
            ]),
            ccp: None,
        };
        assert_eq!(latest_index(&inputs), Some(d("2026-10-28")));
        let empty = benchmarks::BenchmarkInputs {
            stored: vec![],
            history: HashMap::new(),
            ccp: None,
        };
        assert_eq!(latest_index(&empty), None);
    }

    #[test]
    fn test_earliest_input_takes_min_over_index_and_quotes() {
        let master = json!({"price_factor": {"benchmarks": [{"name": "m", "date": "2026-10-09"}]}});
        let inputs = benchmarks::BenchmarkInputs {
            stored: vec![json!({"name": "s", "date": "2026-10-20"})],
            history: HashMap::from([(
                "LJM_QM".to_string(),
                vec![IndexPoint {
                    date: "2025-10-27".into(),
                    price: 1.0,
                }],
            )]),
            ccp: Some(vec![IndexPoint {
                date: "2024-01-01".into(),
                price: 1.0,
            }]),
        };
        // CCP 单独定不出价, 不算输入.
        assert_eq!(earliest_input(&master, &inputs), Some(d("2025-10-27")));
        let empty = benchmarks::BenchmarkInputs {
            stored: vec![],
            history: HashMap::new(),
            ccp: None,
        };
        assert_eq!(earliest_input(&json!({}), &empty), None);
    }

    #[test]
    fn test_missing_by_day_only_lists_gaps() {
        let names = vec!["甲".to_string(), "乙".to_string()];
        let existing = HashSet::from([
            (d("2026-10-01"), "甲".to_string()),
            (d("2026-10-01"), "乙".to_string()),
            (d("2026-10-02"), "甲".to_string()),
        ]);
        let plan = missing_by_day((d("2026-10-01"), d("2026-10-03")), &names, &existing);
        assert_eq!(
            plan,
            vec![
                (d("2026-10-02"), BTreeSet::from(["乙".to_string()])),
                (
                    d("2026-10-03"),
                    BTreeSet::from(["甲".to_string(), "乙".to_string()])
                ),
            ]
        );
    }

    #[test]
    fn test_backfill_rows_price_each_day_as_of_that_day_and_skip_failures() {
        let (mut master, _) = fixture();
        master["price_factor"]["index_series"] = json!([]);
        master["price_factor"]["benchmarks"] = json!([
            {"name": "旧基准", "coal_type": "主焦煤", "props": {"S": 1.0, "Y": 20.0}, "price": 1000.0, "date": "2026-10-02"}
        ]);
        let inputs = benchmarks::BenchmarkInputs {
            stored: vec![
                json!({"name": "新基准", "coal_type": "主焦煤", "props": {"S": 1.0, "Y": 20.0},
                                "price": 2000.0, "date": "2026-10-03", "origin": "stored"}),
            ],
            history: HashMap::new(),
            ccp: None,
        };
        let plan = vec![
            (d("2026-10-01"), BTreeSet::from(["主力".to_string()])), // 一个基准都没有: 跳过
            (d("2026-10-02"), BTreeSet::from(["主力".to_string()])), // 只有旧基准
            (
                d("2026-10-03"),
                BTreeSet::from(["主力".to_string(), "停用".to_string()]),
            ),
        ];
        let (rows, skipped) = backfill_rows(&master, &inputs, &plan);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, d("2026-10-01"));
        let got: Vec<_> = rows
            .iter()
            .map(|(day, row)| (day.to_string(), row.name.clone(), row.price))
            .collect();
        assert_eq!(
            got,
            vec![
                ("2026-10-02".into(), "主力".into(), 1000.0),
                ("2026-10-03".into(), "主力".into(), 2000.0),
                ("2026-10-03".into(), "停用".into(), 2020.0),
            ]
        );
    }

    #[test]
    fn test_backfill_rows_skip_failed_estimate_and_continue() {
        let (mut master, _) = fixture();
        master["price_factor"]["benchmarks"] = json!([
            {"name": "基准", "props": {"Y": 20.0}, "price": -1.0, "date": "2026-10-01"}
        ]);
        let inputs = benchmarks::BenchmarkInputs {
            stored: vec![json!({"name": "基准", "props": {"Y": 20.0},
                               "price": 2000.0, "date": "2026-10-02"})],
            history: HashMap::new(),
            ccp: None,
        };
        let plan = vec![
            (d("2026-10-01"), BTreeSet::from(["主力".to_string()])),
            (d("2026-10-02"), BTreeSet::from(["主力".to_string()])),
        ];
        let (rows, skipped) = backfill_rows(&master, &inputs, &plan);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, d("2026-10-01"));
        assert!(!skipped[0].1.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, d("2026-10-02"));
        assert_eq!(rows[0].1.price, 2000.0);
    }

    #[test]
    fn test_backfill_requires_all_inputs_before_writing() {
        let make_inputs = |ccp| benchmarks::BenchmarkInputs {
            stored: vec![],
            history: HashMap::new(),
            ccp,
        };
        let ccp = || {
            Some(vec![IndexPoint {
                date: "2026-10-01".into(),
                price: 1900.0,
            }])
        };
        for error in ["报价读取失败", "指数读取失败"] {
            assert_eq!(
                require_backfill_inputs(&json!({}), make_inputs(ccp()), Some(error.into()))
                    .err()
                    .as_deref(),
                Some(error)
            );
        }
        assert!(require_backfill_inputs(&json!({}), make_inputs(None), None).is_err());
        assert!(require_backfill_inputs(&json!({}), make_inputs(Some(vec![])), None).is_err());
        assert!(require_backfill_inputs(&json!({}), make_inputs(ccp()), None).is_ok());
    }

    #[test]
    fn test_history_response_shape() {
        let body = json!(CoalPriceHistory {
            ok: true,
            name: "临北".into(),
            points: vec![
                CoalPricePoint {
                    day: d("2025-10-27"),
                    price: 2050.3,
                    benchmark: Some("低硫主焦煤指数".into()),
                    source: "backfill".into()
                },
                CoalPricePoint {
                    day: d("2026-10-30"),
                    price: 2214.0,
                    benchmark: None,
                    source: "live".into()
                },
            ],
        });
        assert_eq!(
            body["points"][0],
            json!({"day": "2025-10-27", "price": 2050.3, "benchmark": "低硫主焦煤指数", "source": "backfill"})
        );
        assert!(body["points"][1]["benchmark"].is_null());
    }
    fn backfill_index_fixture() -> (Value, benchmarks::BenchmarkInputs) {
        let master = json!({"price_factor": {"index_series": [
            {"code": "LJM_DLZJM", "name": "低硫主焦煤指数", "props": {"S": 0.6}},
            {"code": "LJM_FM", "name": "肥煤指数", "props": {"S": 1.0}}
        ]}});
        let inputs = benchmarks::BenchmarkInputs {
            stored: vec![],
            history: HashMap::from([
                (
                    "LJM_DLZJM".into(),
                    vec![IndexPoint {
                        date: "2025-10-27".into(),
                        price: 2000.0,
                    }],
                ),
                (
                    "LJM_FM".into(),
                    vec![IndexPoint {
                        date: "2026-10-01".into(),
                        price: 2100.0,
                    }],
                ),
            ]),
            ccp: Some(vec![IndexPoint {
                date: "2026-10-08".into(),
                price: 1900.0,
            }]),
        };
        (master, inputs)
    }

    #[test]
    fn test_backfill_rejects_missing_or_empty_configured_index() {
        for empty in [false, true] {
            let (master, mut inputs) = backfill_index_fixture();
            if empty {
                inputs.history.insert("LJM_FM".into(), vec![]);
            } else {
                inputs.history.remove("LJM_FM");
            }
            assert!(require_backfill_inputs(&master, inputs, None).is_err());
        }
        let (master, mut inputs) = backfill_index_fixture();
        inputs.history.clear();
        assert!(require_backfill_inputs(&master, inputs, None).is_err());
    }

    #[test]
    fn test_backfill_accepts_nonempty_indexes_with_different_starts_and_old_points() {
        // 首日不同、最新点早于今天都是合法输入, 不能借完整性检查改变按当日可得的口径.
        let (master, inputs) = backfill_index_fixture();
        assert!(require_backfill_inputs(&master, inputs, None).is_ok());
    }
}
