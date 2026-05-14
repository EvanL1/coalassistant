//! 把 blend_kit 嵌入的 master JSON 写入 SQLite.
//!
//! Seed 策略:
//!   - 首次启动: 全量写入 (master_coals + master_indicators + default_contract)
//!   - 后续启动: 增量更新
//!       * 新增煤名 → 插入
//!       * 同名煤 master 字段变化 → 更新 master_indicators (用户的 user_overrides 不动)
//!       * master 删除的煤 → 暂不处理 (避免误删用户引用)
//!   - 默认合同: 只在首次启动时插入, 避免覆盖用户对默认合同的修改
use crate::db::DbError;
use blend_kit::{CoalMaster, CoalMasterEntry, Confidence, DefaultContract, Direction, MasterStatus};
use rusqlite::{params, Connection};

const META_KEY_MASTER_VERSION: &str = "master_version";
const META_KEY_DEFAULT_CONTRACT_SEEDED: &str = "default_contract_seeded";

/// 执行 seed: 从 blend_kit 嵌入的 master JSON 同步到 SQLite.
/// 幂等: 多次调用安全, 不重复插入.
pub fn seed_master(conn: &mut Connection) -> Result<SeedReport, DbError> {
    let master = CoalMaster::load_embedded()
        .map_err(|e| DbError::SeedFailed(format!("加载嵌入 master 失败: {}", e)))?;

    let tx = conn.transaction()?;
    let prev_version: Option<String> = tx
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![META_KEY_MASTER_VERSION],
            |row| row.get(0),
        )
        .ok();

    let mut report = SeedReport {
        master_version: master.version.clone(),
        previous_version: prev_version.clone(),
        coals_inserted: 0,
        coals_updated: 0,
        indicators_written: 0,
        default_contract_inserted: false,
    };

    for entry in &master.coals {
        let r = upsert_coal(&tx, entry, &master.version)?;
        if r.was_insert {
            report.coals_inserted += 1;
        } else if r.was_update {
            report.coals_updated += 1;
        }
        report.indicators_written += r.indicators_written;
    }

    // 默认合同: 仅在首次时插入
    if prev_version.is_none() {
        let already_seeded: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![META_KEY_DEFAULT_CONTRACT_SEEDED],
                |row| row.get(0),
            )
            .ok();
        if already_seeded.is_none() {
            insert_default_contract(&tx, &master.default_contract)?;
            tx.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')",
                params![META_KEY_DEFAULT_CONTRACT_SEEDED],
            )?;
            report.default_contract_inserted = true;
        }
    }

    // 记录 master 版本
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![META_KEY_MASTER_VERSION, master.version],
    )?;

    tx.commit()?;
    Ok(report)
}

struct UpsertResult {
    was_insert: bool,
    was_update: bool,
    indicators_written: usize,
}

fn upsert_coal(
    tx: &rusqlite::Transaction,
    entry: &CoalMasterEntry,
    master_version: &str,
) -> Result<UpsertResult, DbError> {
    let status_str = status_to_str(&entry.status);
    // 先尝试 INSERT, 失败 (主键冲突) 则 UPDATE
    let inserted = tx.execute(
        r#"
        INSERT OR IGNORE INTO master_coals (name, region, coal_type, status, master_version, note)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            entry.name,
            entry.region.as_deref(),
            entry.coal_type.as_deref(),
            status_str,
            master_version,
            entry.note.as_deref()
        ],
    )?;

    let was_insert = inserted == 1;
    let mut was_update = false;

    if !was_insert {
        // 已存在 - 检查是否需要更新元数据
        let n = tx.execute(
            r#"
            UPDATE master_coals
            SET region = ?2, coal_type = ?3, status = ?4, master_version = ?5, note = ?6
            WHERE name = ?1
              AND (region IS NOT ?2 OR coal_type IS NOT ?3 OR status IS NOT ?4 OR note IS NOT ?6)
            "#,
            params![
                entry.name,
                entry.region.as_deref(),
                entry.coal_type.as_deref(),
                status_str,
                master_version,
                entry.note.as_deref()
            ],
        )?;
        was_update = n > 0;
    }

    // 写化验指标 (REPLACE 语义: master 是权威源, 字段值变了就更新)
    let mut indicators_written = 0;
    for (field, value) in &entry.props {
        let confidence = entry.confidence.get(field).map(confidence_to_str);
        tx.execute(
            r#"
            INSERT OR REPLACE INTO master_indicators (coal_name, field, value, confidence)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![entry.name, field, value, confidence],
        )?;
        indicators_written += 1;
    }

    // fob/frt 也写入 master_indicators (作为字段处理, 与 schema 设计一致)
    if let Some(fob) = entry.fob {
        let confidence = entry.confidence.get("fob").map(confidence_to_str);
        tx.execute(
            "INSERT OR REPLACE INTO master_indicators (coal_name, field, value, confidence) VALUES (?1, 'fob', ?2, ?3)",
            params![entry.name, fob, confidence],
        )?;
        indicators_written += 1;
    }
    if let Some(frt) = entry.frt {
        let confidence = entry.confidence.get("frt").map(confidence_to_str);
        tx.execute(
            "INSERT OR REPLACE INTO master_indicators (coal_name, field, value, confidence) VALUES (?1, 'frt', ?2, ?3)",
            params![entry.name, frt, confidence],
        )?;
        indicators_written += 1;
    }

    Ok(UpsertResult {
        was_insert,
        was_update,
        indicators_written,
    })
}

fn insert_default_contract(
    tx: &rusqlite::Transaction,
    contract: &DefaultContract,
) -> Result<(), DbError> {
    let now = current_timestamp();
    tx.execute(
        r#"
        INSERT INTO contracts (name, is_default, is_active, created_at)
        VALUES (?1, 1, 1, ?2)
        "#,
        params![contract.name, now],
    )?;
    let contract_id = tx.last_insert_rowid();

    for spec in &contract.specs {
        let direction_str = direction_to_str(&spec.direction);
        tx.execute(
            r#"
            INSERT INTO contract_specs
                (contract_id, indicator, direction, min_val, max_val, enabled)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                contract_id,
                spec.indicator,
                direction_str,
                spec.min,
                spec.max,
                spec.enabled as i32
            ],
        )?;
    }
    Ok(())
}

fn status_to_str(s: &MasterStatus) -> &'static str {
    match s {
        MasterStatus::Verified => "verified",
        MasterStatus::Active => "active",
        MasterStatus::Draft => "draft",
        MasterStatus::Incomplete => "incomplete",
        MasterStatus::Archived => "archived",
    }
}

fn confidence_to_str(c: &Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn direction_to_str(d: &Direction) -> &'static str {
    match d {
        Direction::Upper => "Upper",
        Direction::Lower => "Lower",
        Direction::Range => "Range",
    }
}

fn current_timestamp() -> String {
    // 用 chrono? 这里避免额外依赖, 用 std::time 输出 ISO8601-ish
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", now)
}

/// seed 报告: 给前端展示 "本次启动从 master 同步了什么".
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeedReport {
    pub master_version: String,
    pub previous_version: Option<String>,
    pub coals_inserted: usize,
    pub coals_updated: usize,
    pub indicators_written: usize,
    pub default_contract_inserted: bool,
}
