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
    pub region: Option<String>,
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
pub fn list_coals(conn: &Connection, status: Option<&str>) -> Result<Vec<CoalView>, DbError> {
    let mut sql = String::from(
        "SELECT name, region, coal_type, status, note FROM master_coals",
    );
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
    for (name, region, coal_type, status, note) in rows {
        let view = build_coal_view(conn, name, region, coal_type, status, note)?;
        out.push(view);
    }
    Ok(out)
}

pub fn get_coal(conn: &Connection, name: &str) -> Result<CoalView, DbError> {
    let row = conn
        .query_row(
            "SELECT name, region, coal_type, status, note FROM master_coals WHERE name = ?1",
            params![name],
            map_coal_meta,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => DbError::NotFound(format!("煤 '{}' 不存在", name)),
            _ => DbError::Sqlite(e),
        })?;
    let (n, region, coal_type, status, note) = row;
    build_coal_view(conn, n, region, coal_type, status, note)
}

fn map_coal_meta(
    row: &rusqlite::Row,
) -> rusqlite::Result<(String, Option<String>, Option<String>, String, Option<String>)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
}

fn build_coal_view(
    conn: &Connection,
    name: String,
    region: Option<String>,
    coal_type: Option<String>,
    status: String,
    note: Option<String>,
) -> Result<CoalView, DbError> {
    let mut fields: HashMap<String, FieldValue> = HashMap::new();

    // 1. 先取 master_indicators 作底
    let mut stmt = conn.prepare(
        "SELECT field, value, confidence FROM master_indicators WHERE coal_name = ?1",
    )?;
    for row in stmt.query_map(params![name], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?, row.get::<_, Option<String>>(2)?))
    })? {
        let (field, value, conf) = row?;
        fields.insert(
            field,
            FieldValue {
                value,
                source: FieldSource::Master,
                confidence: conf,
            },
        );
    }

    // 2. 应用 user_overrides
    let mut stmt = conn.prepare(
        "SELECT field, value FROM user_overrides WHERE coal_name = ?1",
    )?;
    for row in stmt.query_map(params![name], |row| {
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
            params![name],
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

    let (enabled, today_fob, today_frt, price_updated_at, user_note) = match prefs {
        Some(p) => p,
        None => (false, None, None, None, None),
    };

    if let Some(fob) = today_fob {
        fields.insert(
            "fob".into(),
            FieldValue {
                value: fob,
                source: FieldSource::TodayPrice,
                confidence: Some("high".into()),
            },
        );
    }
    if let Some(frt) = today_frt {
        fields.insert(
            "frt".into(),
            FieldValue {
                value: frt,
                source: FieldSource::TodayPrice,
                confidence: Some("high".into()),
            },
        );
    }

    Ok(CoalView {
        name,
        region,
        coal_type,
        status,
        note,
        fields,
        enabled,
        price_updated_at,
        user_note,
    })
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
// 工具
// ============================================================

fn current_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", now)
}
