//! 数据库查询函数. 暴露给 Tauri commands.
//!
//! 关键 view 逻辑:
//!   - CoalView: 合并 master 字段 + user_overrides + user_coal_prefs
//!     字段优先级: today_fob/today_frt > user_overrides[fob/frt] > master_indicators
//!     其他字段优先级: user_overrides > master_indicators
use crate::db::DbError;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 单个字段的合并视图: 当前值 + 来源标记.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldValue {
    pub value: f64,
    pub source: FieldSource,
    pub confidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldSource {
    /// 来自 master, 用户未修改
    Master,
    /// 用户修改过的字段
    UserOverride,
    /// 用户当日录入的临时价格 (今日特价)
    TodayPrice,
}

// IndicatorView 预留给未来分字段 diff 视图; 当前 CoalView.fields 已够用.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorView {
    pub field: String,
    pub current: Option<FieldValue>,
    pub master_value: Option<f64>,
}

/// 完整煤视图: master 字段 + user 改动 + 启用状态.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoalView {
    pub name: String,
    /// 兼容旧前端: province + city 合成 (如 "山西吕梁")
    pub region: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub county: Option<String>,
    pub mine_name: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub coal_type: Option<String>,
    pub status: String,
    pub note: Option<String>,
    /// 8 项化验指标 + fob/frt, 已合并 master + user_overrides + today_price
    pub fields: HashMap<String, FieldValue>,
    pub enabled: bool,
    pub price_updated_at: Option<String>,
    pub user_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecView {
    pub indicator: String,
    pub direction: String,
    pub min_val: Option<f64>,
    pub max_val: Option<f64>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractView {
    pub id: i64,
    pub name: String,
    pub is_default: bool,
    pub is_active: bool,
    pub created_at: String,
    pub specs: Vec<SpecView>,
}

// ============================================================
// 煤池查询
// ============================================================

/// 列出所有煤. status 可过滤 (None = 全部).
/// mines 元数据列 (顺序与 map_coal_meta 对应)
const COAL_META_COLS: &str =
    "name, province, city, county, mine_name, lat, lng, coal_type, status, note";

pub fn list_coals(conn: &Connection, status: Option<&str>) -> Result<Vec<CoalView>, DbError> {
    let mut sql = format!("SELECT {COAL_META_COLS} FROM mines");
    if status.is_some() {
        sql.push_str(" WHERE status = ?1");
    }
    sql.push_str(" ORDER BY status, name");

    let mut stmt = conn.prepare(&sql)?;
    let rows = if let Some(s) = status {
        stmt.query_map(params![s], map_coal_meta)?.collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], map_coal_meta)?.collect::<Result<Vec<_>, _>>()?
    };

    let mut out = Vec::with_capacity(rows.len());
    for meta in rows {
        out.push(build_coal_view(conn, meta)?);
    }
    Ok(out)
}

pub fn get_coal(conn: &Connection, name: &str) -> Result<CoalView, DbError> {
    let meta = conn
        .query_row(
            &format!("SELECT {COAL_META_COLS} FROM mines WHERE name = ?1"),
            params![name],
            map_coal_meta,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => DbError::NotFound(format!("煤 '{}' 不存在", name)),
            _ => DbError::Sqlite(e),
        })?;
    build_coal_view(conn, meta)
}

/// mines 表的元数据行 (不含指标值; 指标在 build_coal_view 里单独读).
struct CoalMeta {
    name: String,
    province: Option<String>,
    city: Option<String>,
    county: Option<String>,
    mine_name: Option<String>,
    lat: Option<f64>,
    lng: Option<f64>,
    coal_type: Option<String>,
    status: String,
    note: Option<String>,
}

fn map_coal_meta(row: &rusqlite::Row) -> rusqlite::Result<CoalMeta> {
    Ok(CoalMeta {
        name: row.get(0)?,
        province: row.get(1)?,
        city: row.get(2)?,
        county: row.get(3)?,
        mine_name: row.get(4)?,
        lat: row.get(5)?,
        lng: row.get(6)?,
        coal_type: row.get(7)?,
        status: row.get(8)?,
        note: row.get(9)?,
    })
}

fn build_coal_view(conn: &Connection, meta: CoalMeta) -> Result<CoalView, DbError> {
    // (mines 列名, 前端 fields key) —— 前端约定指标大写, petro/fob/frt 小写
    const COLS: [(&str, &str); 10] = [
        ("s", "S"), ("a", "A"), ("v", "V"), ("g", "G"), ("y", "Y"),
        ("petro", "petro"), ("csr", "CSR"), ("m", "M"), ("fob", "fob"), ("frt", "frt"),
    ];
    let mut fields: HashMap<String, FieldValue> = HashMap::new();

    // 1. master 底: mines 那一行的指标 + 价格列
    let vals: [Option<f64>; 10] = conn.query_row(
        "SELECT s, a, v, g, y, petro, csr, m, fob, frt FROM mines WHERE name = ?1",
        params![meta.name],
        |r| {
            Ok([
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?,
                r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?,
            ])
        },
    )?;

    // 可信度: mine_field_confidence (列名小写 → confidence)
    let mut conf_map: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT field, confidence FROM mine_field_confidence
             WHERE mine_id = (SELECT id FROM mines WHERE name = ?1)",
        )?;
        for row in stmt.query_map(params![meta.name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (col, conf) = row?;
            conf_map.insert(col, conf);
        }
    }

    for (i, (col, key)) in COLS.iter().enumerate() {
        if let Some(value) = vals[i] {
            fields.insert(
                (*key).to_string(),
                FieldValue {
                    value,
                    source: FieldSource::Master,
                    confidence: conf_map.get(*col).cloned(),
                },
            );
        }
    }

    // 2. 应用 user_overrides
    let mut stmt = conn.prepare("SELECT field, value FROM user_overrides WHERE coal_name = ?1")?;
    for row in stmt.query_map(params![meta.name], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })? {
        let (field, value) = row?;
        fields.insert(
            field,
            FieldValue {
                value,
                source: FieldSource::UserOverride,
                confidence: Some("high".into()), // 用户改的算 high
            },
        );
    }

    // 3. 应用今日特价 (today_fob/today_frt)
    let prefs: Option<(bool, Option<f64>, Option<f64>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT enabled, today_fob, today_frt, price_updated_at, note
             FROM user_coal_prefs WHERE coal_name = ?1",
            params![meta.name],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? != 0,
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .ok();

    let (enabled, today_fob, today_frt, price_updated_at, user_note) =
        prefs.unwrap_or((false, None, None, None, None));

    if let Some(fob) = today_fob {
        fields.insert(
            "fob".into(),
            FieldValue { value: fob, source: FieldSource::TodayPrice, confidence: Some("high".into()) },
        );
    }
    if let Some(frt) = today_frt {
        fields.insert(
            "frt".into(),
            FieldValue { value: frt, source: FieldSource::TodayPrice, confidence: Some("high".into()) },
        );
    }

    let region = synth_region(meta.province.as_deref(), meta.city.as_deref());
    Ok(CoalView {
        name: meta.name,
        region,
        province: meta.province,
        city: meta.city,
        county: meta.county,
        mine_name: meta.mine_name,
        lat: meta.lat,
        lng: meta.lng,
        coal_type: meta.coal_type,
        status: meta.status,
        note: meta.note,
        fields,
        enabled,
        price_updated_at,
        user_note,
    })
}

/// province + city 合成旧式 region 字符串, 兼容前端.
fn synth_region(province: Option<&str>, city: Option<&str>) -> Option<String> {
    match (province, city) {
        (Some(p), Some(c)) => Some(format!("{p}{c}")),
        (Some(p), None) => Some(p.to_string()),
        (None, Some(c)) => Some(c.to_string()),
        (None, None) => None,
    }
}

// ============================================================
// 用户层修改
// ============================================================

pub fn set_user_override(
    conn: &mut Connection,
    coal_name: &str,
    field: &str,
    value: f64,
) -> Result<(), DbError> {
    let now = current_timestamp();
    conn.execute(
        r#"
        INSERT OR REPLACE INTO user_overrides (coal_name, field, value, updated_at)
        VALUES (?1, ?2, ?3, ?4)
        "#,
        params![coal_name, field, value, now],
    )?;
    Ok(())
}

pub fn clear_user_override(
    conn: &mut Connection,
    coal_name: &str,
    field: &str,
) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM user_overrides WHERE coal_name = ?1 AND field = ?2",
        params![coal_name, field],
    )?;
    Ok(())
}

pub fn set_today_price(
    conn: &mut Connection,
    coal_name: &str,
    fob: Option<f64>,
    frt: Option<f64>,
) -> Result<(), DbError> {
    let now = current_timestamp();
    conn.execute(
        r#"
        INSERT INTO user_coal_prefs (coal_name, enabled, today_fob, today_frt, price_updated_at)
        VALUES (?1, 0, ?2, ?3, ?4)
        ON CONFLICT(coal_name) DO UPDATE SET
            today_fob = excluded.today_fob,
            today_frt = excluded.today_frt,
            price_updated_at = excluded.price_updated_at
        "#,
        params![coal_name, fob, frt, now],
    )?;
    Ok(())
}

pub fn set_enabled(conn: &mut Connection, coal_name: &str, enabled: bool) -> Result<(), DbError> {
    conn.execute(
        r#"
        INSERT INTO user_coal_prefs (coal_name, enabled)
        VALUES (?1, ?2)
        ON CONFLICT(coal_name) DO UPDATE SET enabled = excluded.enabled
        "#,
        params![coal_name, enabled as i32],
    )?;
    Ok(())
}

// ============================================================
// 合同查询
// ============================================================

pub fn list_contracts(conn: &Connection) -> Result<Vec<ContractView>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, is_default, is_active, created_at FROM contracts ORDER BY id",
    )?;
    let contracts: Vec<(i64, String, bool, bool, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, i64>(3)? != 0,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let mut out = Vec::with_capacity(contracts.len());
    for (id, name, is_default, is_active, created_at) in contracts {
        let specs = list_specs(conn, id)?;
        out.push(ContractView {
            id,
            name,
            is_default,
            is_active,
            created_at,
            specs,
        });
    }
    Ok(out)
}

fn list_specs(conn: &Connection, contract_id: i64) -> Result<Vec<SpecView>, DbError> {
    let mut stmt = conn.prepare(
        r#"SELECT indicator, direction, min_val, max_val, enabled
           FROM contract_specs WHERE contract_id = ?1
           ORDER BY indicator"#,
    )?;
    let specs = stmt
        .query_map(params![contract_id], |row| {
            Ok(SpecView {
                indicator: row.get(0)?,
                direction: row.get(1)?,
                min_val: row.get(2)?,
                max_val: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(specs)
}

pub fn get_active_contract(conn: &Connection) -> Result<ContractView, DbError> {
    let (id, name, is_default, is_active, created_at): (i64, String, bool, bool, String) = conn
        .query_row(
            "SELECT id, name, is_default, is_active, created_at FROM contracts WHERE is_active = 1 LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0, row.get::<_, i64>(3)? != 0, row.get(4)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => DbError::NotFound("无激活合同".into()),
            _ => DbError::Sqlite(e),
        })?;
    let specs = list_specs(conn, id)?;
    Ok(ContractView { id, name, is_default, is_active, created_at, specs })
}

// ============================================================
// 历史方案 (采集 + 回填实测 CSR)
// ============================================================

/// 一条历史方案记录. result_json 内含混合后指标(回归 X), csr_measured 是回填的实测 CSR(回归 y).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: i64,
    pub occurred_at: String,
    pub contract_name: String,
    pub cost_cif: f64,
    pub result_json: String,
    pub csr_measured: Option<f64>,
}

/// 保存一次配煤方案, 返回新行 id. occurred_at 由前端给 (ISO8601, 两端统一).
pub fn save_history(
    conn: &mut Connection,
    occurred_at: &str,
    contract_name: &str,
    cost_cif: f64,
    total_quantity: Option<f64>,
    result_json: &str,
) -> Result<i64, DbError> {
    conn.execute(
        r#"
        INSERT INTO blend_history (occurred_at, contract_name, total_quantity, cost_cif, result_json)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![occurred_at, contract_name, total_quantity, cost_cif, result_json],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 列出历史方案, 倒序 (最新在前).
pub fn list_history(conn: &Connection) -> Result<Vec<HistoryRecord>, DbError> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, occurred_at, contract_name, cost_cif, result_json, csr_measured
        FROM blend_history
        ORDER BY occurred_at DESC, id DESC
        "#,
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(HistoryRecord {
                id: row.get(0)?,
                occurred_at: row.get(1)?,
                contract_name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                cost_cif: row.get(3)?,
                result_json: row.get(4)?,
                csr_measured: row.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// 清空所有历史方案.
pub fn clear_history(conn: &mut Connection) -> Result<(), DbError> {
    conn.execute("DELETE FROM blend_history", [])?;
    Ok(())
}

/// 回填某条记录的实测 CSR. id 不存在 → NotFound.
pub fn set_measured_csr(conn: &mut Connection, id: i64, csr_measured: f64) -> Result<(), DbError> {
    let n = conn.execute(
        "UPDATE blend_history SET csr_measured = ?2 WHERE id = ?1",
        params![id, csr_measured],
    )?;
    if n == 0 {
        return Err(DbError::NotFound(format!("blend_history id={id}")));
    }
    Ok(())
}

// ============================================================
// 工具
// ============================================================

fn current_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", now)
}
