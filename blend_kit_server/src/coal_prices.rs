//! 共享煤库的每日估价快照; 用户修改的化验值只由 /api/price 现算.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{
    benchmarks, database_error, is_authenticated, mcci, merged_master_json_from_pool,
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

/// 删写在同一个事务内, 插入失败则原快照完整保留.
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

/// interval 首次立即触发; 不受 MCCI_FETCH 开关影响, 每轮 panic 由子任务隔离.
pub(crate) fn spawn_hourly(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        loop {
            ticker.tick().await;
            let pool = pool.clone();
            let round = tokio::spawn(async move { refresh(&pool).await });
            match round.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!(%error, "每日估价快照更新失败, 保留旧数据"),
                Err(error) => tracing::error!(%error, "每日估价快照这一轮异常退出, 下个小时再来"),
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
