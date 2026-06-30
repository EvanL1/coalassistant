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
use rusqlite::{params, Connection, OptionalExtension};

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
        let r = upsert_coal(&tx, entry)?;
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
) -> Result<UpsertResult, DbError> {
    let status_str = status_to_str(&entry.status);
    let (province, city) = split_region(entry.region.as_deref());
    let p = |k: &str| entry.props.get(k).copied();

    // 是否已存在 (仅用于 SeedReport 的 insert/update 统计)
    let was_insert = tx
        .query_row("SELECT 1 FROM mines WHERE name = ?1", params![entry.name], |_| Ok(()))
        .optional()?
        .is_none();

    // 宽表 upsert: 新增→插入, 已存在→更新 master 字段.
    // 用 ON CONFLICT DO UPDATE (而非 INSERT OR REPLACE): 不换 id, 不触发级联删用户数据.
    // UPDATE 子句刻意不含 county/mine_name/lat/lng → 用户后补的位置不被每次 seed 刷成 NULL.
    tx.execute(
        r#"
        INSERT INTO mines
            (name, coal_type, status, province, city,
             s, a, v, g, y, petro, csr, m, fob, frt, note)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
        ON CONFLICT(name) DO UPDATE SET
            coal_type = excluded.coal_type,
            status    = excluded.status,
            province  = excluded.province,
            city      = excluded.city,
            s = excluded.s, a = excluded.a, v = excluded.v, g = excluded.g,
            y = excluded.y, petro = excluded.petro, csr = excluded.csr, m = excluded.m,
            fob = excluded.fob, frt = excluded.frt,
            note = excluded.note
        "#,
        params![
            entry.name,
            entry.coal_type.as_deref(),
            status_str,
            province,
            city,
            p("S"), p("A"), p("V"), p("G"), p("Y"), p("petro"), p("CSR"), p("M"),
            entry.fob,
            entry.frt,
            entry.note.as_deref(),
        ],
    )?;

    let mine_id: i64 =
        tx.query_row("SELECT id FROM mines WHERE name = ?1", params![entry.name], |r| r.get(0))?;

    // 每字段可信度 (master 权威, upsert 语义)
    let mut indicators_written = 0;
    for (field, conf) in &entry.confidence {
        let Some(col) = field_to_col(field) else { continue };
        tx.execute(
            r#"
            INSERT INTO mine_field_confidence (mine_id, field, confidence)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(mine_id, field) DO UPDATE SET confidence = excluded.confidence
            "#,
            params![mine_id, col, confidence_to_str(conf)],
        )?;
        indicators_written += 1;
    }

    Ok(UpsertResult {
        was_insert,
        was_update: !was_insert,
        indicators_written,
    })
}

/// master 字段名 (大写 S/A/.../petro/CSR/M + fob/frt) → mines 列名 (小写).
fn field_to_col(field: &str) -> Option<&'static str> {
    Some(match field {
        "S" => "s",
        "A" => "a",
        "V" => "v",
        "G" => "g",
        "Y" => "y",
        "petro" => "petro",
        "CSR" => "csr",
        "M" => "m",
        "fob" => "fob",
        "frt" => "frt",
        _ => return None,
    })
}

/// 把 master 的 region ("山西吕梁") 拆成 (province, city).
/// 匹配不到已知省名时, 整串作为 city, province 留空.
fn split_region(region: Option<&str>) -> (Option<String>, Option<String>) {
    const PROVINCES: &[&str] = &[
        "内蒙古", "黑龙江", "山西", "陕西", "河北", "河南", "山东", "宁夏",
        "新疆", "甘肃", "青海", "贵州", "云南", "四川", "安徽", "辽宁", "吉林", "重庆",
    ];
    let r = match region {
        Some(r) if !r.is_empty() => r,
        _ => return (None, None),
    };
    for prov in PROVINCES {
        if let Some(rest) = r.strip_prefix(prov) {
            let city = rest.trim();
            return (Some((*prov).to_string()), (!city.is_empty()).then(|| city.to_string()));
        }
    }
    (None, Some(r.to_string()))
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
