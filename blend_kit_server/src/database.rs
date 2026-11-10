use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

use crate::master_data::{CoalUpdate, StoredOverride};

const DEFAULT_QUANTITY: f64 = 3700.0;
/// 一铲默认吨数: 常见 5 吨装载机 (3 方斗) 装炼焦精煤约 3 吨.
pub const DEFAULT_SHOVEL_TONS: f64 = 3.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserStorage {
    #[serde(default)]
    pub initialized: bool,
    #[serde(default = "empty_object")]
    pub coal_prefs: Value,
    #[serde(default)]
    pub contract: Option<Value>,
    #[serde(default = "default_quantity")]
    pub quantity: f64,
    #[serde(default = "empty_array")]
    pub user_coals: Value,
    /// 一铲多少吨 (现场铲车配煤的最小单位), 求解时每种煤至少一铲、按整铲配.
    /// None = 没设过 (按 [`DEFAULT_SHOVEL_TONS`]); 上线前的老前端 PUT 时也不带它,
    /// 保存时不能拿 None 冲掉已设的值 (见 save_storage 的 COALESCE).
    #[serde(default)]
    pub shovel_tons: Option<f64>,
}

impl Default for UserStorage {
    fn default() -> Self {
        Self {
            initialized: false,
            coal_prefs: empty_object(),
            contract: None,
            quantity: DEFAULT_QUANTITY,
            user_coals: empty_array(),
            shovel_tons: None,
        }
    }
}

impl UserStorage {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.coal_prefs.is_object() {
            return Err("coal_prefs 必须是 JSON 对象");
        }
        if self
            .contract
            .as_ref()
            .is_some_and(|value| !value.is_array())
        {
            return Err("contract 必须是 JSON 数组或 null");
        }
        if !self.quantity.is_finite() || self.quantity <= 0.0 {
            return Err("quantity 必须是正数");
        }
        if !self.user_coals.is_array() {
            return Err("user_coals 必须是 JSON 数组");
        }
        if self
            .shovel_tons
            .is_some_and(|tons| !tons.is_finite() || tons <= 0.0)
        {
            return Err("shovel_tons 必须是正数或 null");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateHistory {
    pub result: Value,
    pub contract_name: String,
    pub quantity: Option<f64>,
}

impl CreateHistory {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.result.is_object() {
            return Err("result 必须是 JSON 对象");
        }
        if self.contract_name.trim().is_empty() {
            return Err("contract_name 不能为空");
        }
        if self
            .quantity
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("quantity 必须是正数或 null");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MeasuredQuality {
    pub s: Option<f64>,
    pub a: Option<f64>,
    pub v: Option<f64>,
    pub g: Option<f64>,
    pub y: Option<f64>,
    pub m: Option<f64>,
    pub csr: Option<f64>,
    /// 焦炭反应性与冷态强度 (%), 选填.
    pub cri: Option<f64>,
    pub m40: Option<f64>,
    pub m10: Option<f64>,
    /// 炼焦条件, 选填: 装煤密度 (t/m³)、结焦时间 (小时)、炉温 (℃, 标准火道温度).
    pub bulk_density: Option<f64>,
    pub coking_hours: Option<f64>,
    pub flue_temp: Option<f64>,
}

impl MeasuredQuality {
    pub fn validate(&self) -> Result<(), &'static str> {
        const PERCENT: (f64, f64, &str) = (0.0, 100.0, "实测值必须在 0 到 100 之间");
        let checks = [
            (self.s, PERCENT),
            (self.a, PERCENT),
            (self.v, PERCENT),
            (self.g, PERCENT),
            (self.y, PERCENT),
            (self.m, PERCENT),
            (self.csr, PERCENT),
            (self.cri, PERCENT),
            (self.m40, PERCENT),
            (self.m10, PERCENT),
            (self.bulk_density, (0.5, 1.5, "装煤密度应在 0.5~1.5 t/m³")),
            (self.coking_hours, (10.0, 40.0, "结焦时间应在 10~40 小时")),
            (self.flue_temp, (900.0, 1500.0, "炉温应在 900~1500 ℃")),
        ];
        if checks.iter().all(|(value, _)| value.is_none()) {
            return Err("至少提供一项实测值");
        }
        for (value, (minimum, maximum, message)) in checks {
            if value.is_some_and(|v| !v.is_finite() || v <= minimum || v > maximum) {
                return Err(message);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct ImportHistory {
    pub entries: Vec<LegacyHistory>,
}

#[derive(Debug, Deserialize)]
pub struct LegacyHistory {
    pub id: String,
    pub occurred_at: String,
    pub cost_cif: f64,
    #[serde(default)]
    pub recipe: Value,
    pub contract_name: String,
    pub result: Option<Value>,
    pub csr_measured: Option<f64>,
    pub s_measured: Option<f64>,
    pub a_measured: Option<f64>,
    pub v_measured: Option<f64>,
    pub g_measured: Option<f64>,
    pub y_measured: Option<f64>,
    pub m_measured: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct HistoryRecord {
    pub id: String,
    pub occurred_at: DateTime<Utc>,
    pub contract_name: String,
    pub cost_cif: f64,
    pub recipe: Value,
    pub result: Option<Value>,
    pub csr_measured: Option<f64>,
    pub s_measured: Option<f64>,
    pub a_measured: Option<f64>,
    pub v_measured: Option<f64>,
    pub g_measured: Option<f64>,
    pub y_measured: Option<f64>,
    pub m_measured: Option<f64>,
    pub cri_measured: Option<f64>,
    pub m40_measured: Option<f64>,
    pub m10_measured: Option<f64>,
    pub bulk_density: Option<f64>,
    pub coking_hours: Option<f64>,
    pub flue_temp: Option<f64>,
}

pub async fn connect_from_env() -> Option<PgPool> {
    let on_railway = std::env::var_os("RAILWAY_ENVIRONMENT_NAME").is_some();
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ if on_railway => panic!("Railway 部署必须设置 DATABASE_URL"),
        _ => return None,
    };

    let pool = retry_startup(&STARTUP_RETRY_DELAYS, || {
        pool_options().connect(&database_url)
    })
    .await
    .unwrap_or_else(|error| panic!("无法连接 PostgreSQL: {error}"));
    migrator()
        .run(&pool)
        .await
        .unwrap_or_else(|error| panic!("数据库迁移失败: {error}"));
    Some(pool)
}

/// 取连接最多等这么久: 数据库出问题时请求快速失败, 不挂满默认的 30 秒.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(ACQUIRE_TIMEOUT)
}

/// 启动时首次连库失败后的重试间隔. 每次尝试仍受 [`ACQUIRE_TIMEOUT`] 约束, 合计最多等一分多钟:
/// 数据库冷启动或跨区建连慢时不至于直接退出 (Railway 只重启几次).
const STARTUP_RETRY_DELAYS: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
    Duration::from_secs(15),
];

/// 反复尝试 `attempt`, 每次失败后按 `delays` 依次等待; 间隔用完仍失败, 返回最后一次的错误.
async fn retry_startup<T, E, F, Fut>(delays: &[Duration], mut attempt: F) -> Result<T, E>
where
    E: std::fmt::Display,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut delays = delays.iter();
    loop {
        let error = match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let Some(delay) = delays.next() else {
            return Err(error);
        };
        tracing::warn!(%error, retry_in = ?delay, "连接 PostgreSQL 失败, 稍后重试");
        tokio::time::sleep(*delay).await;
    }
}

/// 容忍库里有本二进制不认识的已应用迁移: 回滚到旧镜像时新版迁移已经跑过, 默认设置会
/// 报 VersionMissing 让服务起不来. 前提是迁移只做加法、兼容上一版代码 (见部署文档).
fn migrator() -> sqlx::migrate::Migrator {
    let mut migrator = sqlx::migrate!();
    migrator.set_ignore_missing(true);
    migrator
}

pub async fn health(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

pub async fn load_storage(pool: &PgPool, username: &str) -> Result<UserStorage, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT coal_prefs, contract, quantity, user_coals, shovel_tons
        FROM web_user_state
        WHERE username = $1
        "#,
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;

    Ok(match row {
        Some(row) => UserStorage {
            initialized: true,
            coal_prefs: row.try_get("coal_prefs")?,
            contract: row.try_get("contract")?,
            quantity: row.try_get("quantity")?,
            user_coals: row.try_get("user_coals")?,
            shovel_tons: row.try_get("shovel_tons")?,
        },
        None => UserStorage::default(),
    })
}

pub async fn save_storage(
    pool: &PgPool,
    username: &str,
    storage: &UserStorage,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO web_user_state (
            username, coal_prefs, contract, quantity, user_coals, shovel_tons, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (username) DO UPDATE SET
            coal_prefs = EXCLUDED.coal_prefs,
            contract = EXCLUDED.contract,
            quantity = EXCLUDED.quantity,
            user_coals = EXCLUDED.user_coals,
            shovel_tons = COALESCE(EXCLUDED.shovel_tons, web_user_state.shovel_tons),
            updated_at = NOW()
        "#,
    )
    .bind(username)
    .bind(&storage.coal_prefs)
    .bind(&storage.contract)
    .bind(storage.quantity)
    .bind(&storage.user_coals)
    .bind(storage.shovel_tons)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_history(
    pool: &PgPool,
    username: &str,
    input: &CreateHistory,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let cost_cif = input
        .result
        .pointer("/cost/cif_per_ton")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let recipe = input
        .result
        .get("recipe")
        .cloned()
        .unwrap_or_else(empty_object);

    sqlx::query(
        r#"
        INSERT INTO web_blend_history (
            id, username, occurred_at, contract_name, cost_cif,
            total_quantity, recipe_json, result_json
        )
        VALUES ($1, $2, NOW(), $3, $4, $5, $6, $7)
        "#,
    )
    .bind(&id)
    .bind(username)
    .bind(input.contract_name.trim())
    .bind(cost_cif)
    .bind(input.quantity)
    .bind(recipe)
    .bind(&input.result)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn import_history(
    pool: &PgPool,
    username: &str,
    input: &ImportHistory,
) -> Result<u64, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let mut imported = 0;
    for entry in input.entries.iter().take(100) {
        if entry.id.trim().is_empty()
            || entry.contract_name.trim().is_empty()
            || !entry.cost_cif.is_finite()
            || !entry.recipe.is_object()
        {
            continue;
        }
        let occurred_at = DateTime::parse_from_rfc3339(&entry.occurred_at)
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let result = sqlx::query(
            r#"
            INSERT INTO web_blend_history (
                id, username, occurred_at, contract_name, cost_cif,
                recipe_json, result_json, csr_measured, s_measured,
                a_measured, v_measured, g_measured, y_measured, m_measured
            )
            VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14
            )
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(entry.id.trim())
        .bind(username)
        .bind(occurred_at)
        .bind(entry.contract_name.trim())
        .bind(entry.cost_cif)
        .bind(&entry.recipe)
        .bind(&entry.result)
        .bind(entry.csr_measured)
        .bind(entry.s_measured)
        .bind(entry.a_measured)
        .bind(entry.v_measured)
        .bind(entry.g_measured)
        .bind(entry.y_measured)
        .bind(entry.m_measured)
        .execute(&mut *transaction)
        .await?;
        imported += result.rows_affected();
    }
    transaction.commit().await?;
    Ok(imported)
}

pub async fn count_history(pool: &PgPool, username: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM web_blend_history WHERE username = $1")
        .bind(username)
        .fetch_one(pool)
        .await
}

pub async fn list_history(
    pool: &PgPool,
    username: &str,
) -> Result<Vec<HistoryRecord>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            id, occurred_at, contract_name, cost_cif, recipe_json, result_json,
            csr_measured, s_measured, a_measured, v_measured, g_measured,
            y_measured, m_measured, cri_measured, m40_measured, m10_measured,
            bulk_density, coking_hours, flue_temp
        FROM web_blend_history
        WHERE username = $1
        ORDER BY occurred_at DESC
        LIMIT 100
        "#,
    )
    .bind(username)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(HistoryRecord {
                id: row.try_get("id")?,
                occurred_at: row.try_get("occurred_at")?,
                contract_name: row.try_get("contract_name")?,
                cost_cif: row.try_get("cost_cif")?,
                recipe: row.try_get("recipe_json")?,
                result: row.try_get("result_json")?,
                csr_measured: row.try_get("csr_measured")?,
                s_measured: row.try_get("s_measured")?,
                a_measured: row.try_get("a_measured")?,
                v_measured: row.try_get("v_measured")?,
                g_measured: row.try_get("g_measured")?,
                y_measured: row.try_get("y_measured")?,
                m_measured: row.try_get("m_measured")?,
                cri_measured: row.try_get("cri_measured")?,
                m40_measured: row.try_get("m40_measured")?,
                m10_measured: row.try_get("m10_measured")?,
                bulk_density: row.try_get("bulk_density")?,
                coking_hours: row.try_get("coking_hours")?,
                flue_temp: row.try_get("flue_temp")?,
            })
        })
        .collect()
}

pub async fn set_measured_quality(
    pool: &PgPool,
    username: &str,
    id: &str,
    measured: &MeasuredQuality,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE web_blend_history SET
            s_measured = COALESCE($3, s_measured),
            a_measured = COALESCE($4, a_measured),
            v_measured = COALESCE($5, v_measured),
            g_measured = COALESCE($6, g_measured),
            y_measured = COALESCE($7, y_measured),
            m_measured = COALESCE($8, m_measured),
            csr_measured = COALESCE($9, csr_measured),
            cri_measured = COALESCE($10, cri_measured),
            m40_measured = COALESCE($11, m40_measured),
            m10_measured = COALESCE($12, m10_measured),
            bulk_density = COALESCE($13, bulk_density),
            coking_hours = COALESCE($14, coking_hours),
            flue_temp = COALESCE($15, flue_temp)
        WHERE id = $1 AND username = $2
        "#,
    )
    .bind(id)
    .bind(username)
    .bind(measured.s)
    .bind(measured.a)
    .bind(measured.v)
    .bind(measured.g)
    .bind(measured.y)
    .bind(measured.m)
    .bind(measured.csr)
    .bind(measured.cri)
    .bind(measured.m40)
    .bind(measured.m10)
    .bind(measured.bulk_density)
    .bind(measured.coking_hours)
    .bind(measured.flue_temp)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn clear_history(pool: &PgPool, username: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM web_blend_history WHERE username = $1")
        .bind(username)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

pub fn count_response(count: i64) -> Value {
    json!({ "count": count })
}

/// 管理端列表里的一把密钥. 不含明文, 也不含哈希.
#[derive(Debug, Serialize)]
pub struct ApiKeyRow {
    pub id: String,
    pub name: String,
    /// 用途: data = 写煤库, mcp = 只读 + 求解.
    pub scope: String,
    pub key_prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

/// 新建一把密钥, 只落哈希. 明文由调用方持有并返回给管理端一次.
pub async fn create_api_key(
    pool: &PgPool,
    name: &str,
    scope: &str,
    hash: &str,
    prefix: &str,
) -> Result<ApiKeyRow, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let row = sqlx::query(
        r#"
        INSERT INTO data_api_keys (id, name, scope, key_hash, key_prefix)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, name, scope, key_prefix, created_at, last_used_at, revoked_at
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(scope)
    .bind(hash)
    .bind(prefix)
    .fetch_one(pool)
    .await?;
    api_key_row(row)
}

/// 列出全部密钥 (含已销毁的, 用于追溯历史归属). 永远不返回哈希或明文.
pub async fn list_api_keys(pool: &PgPool) -> Result<Vec<ApiKeyRow>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, scope, key_prefix, created_at, last_used_at, revoked_at
        FROM data_api_keys
        ORDER BY revoked_at IS NOT NULL, created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(api_key_row).collect()
}

/// 销毁一把密钥. 软删除: 保留行, 只置 revoked_at, 以免历史 updated_by 失去出处.
/// 已销毁的再销毁返回 0, 让调用方能区分"刚销毁"和"本就不存在".
pub async fn revoke_api_key(pool: &PgPool, id: &str) -> Result<u64, sqlx::Error> {
    // 非法 UUID 直接当作"不存在", 不必区分 —— 调用方拿到的都是 404.
    if Uuid::parse_str(id).is_err() {
        return Ok(0);
    }
    let result = sqlx::query(
        "UPDATE data_api_keys SET revoked_at = NOW() WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// 用哈希查一把未销毁的密钥, 命中则返回它的名字并刷新 last_used_at.
///
/// 查询按 key_hash 走索引: 攻击者控制的是明文, 拿不到哈希的比较时序信息
/// (要利用得先有 SHA-256 的原像)。
pub async fn authenticate_api_key(
    pool: &PgPool,
    hash: &str,
    scope: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        UPDATE data_api_keys SET last_used_at = NOW()
        WHERE key_hash = $1 AND scope = $2 AND revoked_at IS NULL
        RETURNING name
        "#,
    )
    .bind(hash)
    .bind(scope)
    .fetch_optional(pool)
    .await?;
    row.map(|row| row.try_get("name")).transpose()
}

fn api_key_row(row: sqlx::postgres::PgRow) -> Result<ApiKeyRow, sqlx::Error> {
    let id: String = row.try_get("id")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let last_used_at: Option<DateTime<Utc>> = row.try_get("last_used_at")?;
    let revoked_at: Option<DateTime<Utc>> = row.try_get("revoked_at")?;
    Ok(ApiKeyRow {
        id,
        name: row.try_get("name")?,
        scope: row.try_get("scope")?,
        key_prefix: row.try_get("key_prefix")?,
        created_at: created_at.to_rfc3339(),
        last_used_at: last_used_at.map(|t| t.to_rfc3339()),
        revoked_at: revoked_at.map(|t| t.to_rfc3339()),
    })
}

/// 读出全部煤库覆盖层, 供 GET /api/master 叠加到基线上.
pub async fn list_coal_overrides(pool: &PgPool) -> Result<Vec<StoredOverride>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT coal_name, field, value, source, confidence, updated_at
        FROM coal_overrides
        ORDER BY coal_name, field
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| -> Result<StoredOverride, sqlx::Error> {
            let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
            Ok(StoredOverride {
                coal_name: row.try_get("coal_name")?,
                field: row.try_get("field")?,
                value: row.try_get("value")?,
                source: row.try_get("source")?,
                confidence: row.try_get("confidence")?,
                updated_at: updated_at.to_rfc3339(),
            })
        })
        .collect()
}

/// 整批 upsert 覆盖层. 走事务: 校验已在应用层整批做过, 落库也要么全成要么全不成.
pub async fn upsert_coal_overrides(
    pool: &PgPool,
    updates: &[CoalUpdate],
    updated_by: &str,
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mut affected = 0;
    for u in updates {
        let result = sqlx::query(
            r#"
            INSERT INTO coal_overrides (coal_name, field, value, source, confidence, updated_by)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (coal_name, field) DO UPDATE SET
                value      = EXCLUDED.value,
                source     = EXCLUDED.source,
                confidence = EXCLUDED.confidence,
                updated_by = EXCLUDED.updated_by,
                updated_at = NOW()
            "#,
        )
        .bind(&u.coal)
        .bind(&u.field)
        .bind(u.value)
        .bind(&u.source)
        .bind(&u.confidence)
        .bind(updated_by)
        .execute(&mut *tx)
        .await?;
        affected += result.rows_affected();
    }
    tx.commit().await?;
    Ok(affected)
}

/// 删除一条覆盖 = 把该指标回滚到基线状态.
pub async fn delete_coal_override(
    pool: &PgPool,
    coal: &str,
    field: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM coal_overrides WHERE coal_name = $1 AND field = $2")
        .bind(coal)
        .bind(field)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

fn default_quantity() -> f64 {
    DEFAULT_QUANTITY
}

fn empty_object() -> Value {
    json!({})
}

fn empty_array() -> Value {
    json!([])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_fails_fast_and_migrator_tolerates_newer_migrations() {
        assert_eq!(pool_options().get_acquire_timeout(), ACQUIRE_TIMEOUT);
        assert!(ACQUIRE_TIMEOUT <= Duration::from_secs(10));
        assert!(migrator().ignore_missing);
    }

    /// 按部署文档"回滚到这一版之前的镜像"的步骤走一遍: 旧二进制 (没有 0014、默认不容忍未知迁移)
    /// 先起不来; 执行文档里的 SQL 后能起来, 临时快照清零; 之后再发布本版, 0014 能重新执行.
    /// 每次"启动"用一个新连接池并在用完后关掉, 同进程退出: 迁移失败时 sqlx 不释放咨询锁.
    #[tokio::test]
    #[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
    async fn test_documented_rollback_lets_a_pre_0014_binary_start() {
        let url = std::env::var("DATABASE_URL").expect("请设置独立测试库 DATABASE_URL");
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("rollback_test_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let connect = || {
            let schema = schema.clone();
            PgPoolOptions::new()
                .max_connections(1)
                .after_connect(move |connection, _| {
                    let schema = schema.clone();
                    Box::pin(async move {
                        sqlx::query("SELECT set_config('search_path', $1, false)")
                            .bind(schema)
                            .execute(connection)
                            .await?;
                        Ok(())
                    })
                })
                .connect(&url)
        };
        let old_migrations = tempfile::tempdir().unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for entry in std::fs::read_dir(&source).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name.as_str() < "0014" {
                std::fs::copy(&path, old_migrations.path().join(&name)).unwrap();
            }
        }
        let old = sqlx::migrate::Migrator::new(old_migrations.path())
            .await
            .unwrap();

        let steps = async {
            let current = connect().await?;
            migrator().run(&current).await?;
            sqlx::query(
                "INSERT INTO coal_price_daily (day, name, price, points, source)
                 VALUES ('2026-11-01', '测试煤', 2000, 1000, 'partial')",
            )
            .execute(&current)
            .await?;
            current.close().await;

            let rollback = connect().await?;
            let refused = old.run(&rollback).await.is_err();
            rollback.close().await;

            let console = connect().await?;
            sqlx::query("UPDATE coal_price_daily SET source = 'live' WHERE source = 'partial'")
                .execute(&console)
                .await?;
            sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 14")
                .execute(&console)
                .await?;
            console.close().await;

            let rollback = connect().await?;
            let old_started = old.run(&rollback).await.map_err(|error| error.to_string());
            let partial: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM coal_price_daily WHERE source = 'partial'",
            )
            .fetch_one(&rollback)
            .await?;
            rollback.close().await;

            let redeploy = connect().await?;
            let redeployed = migrator()
                .run(&redeploy)
                .await
                .map_err(|error| error.to_string());
            redeploy.close().await;
            Ok::<_, sqlx::Error>((refused, old_started, partial, redeployed))
        }
        .await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
        let (refused, old_started, partial, redeployed) = steps.unwrap();
        assert!(refused, "旧二进制应因不认识 0014 拒绝启动");
        assert_eq!(old_started, Ok(()));
        assert_eq!(partial, 0);
        assert_eq!(redeployed, Ok(()));
    }

    /// 启动时首次连库: 重试的间隔加起来要有几十秒, 冷启动或跨区建连慢也等得到.
    #[test]
    fn test_startup_connect_waits_tens_of_seconds_in_total() {
        let slept: Duration = STARTUP_RETRY_DELAYS.iter().sum();
        let worst = slept + ACQUIRE_TIMEOUT * (STARTUP_RETRY_DELAYS.len() as u32 + 1);
        assert!(slept >= Duration::from_secs(30), "{slept:?}");
        assert!(worst <= Duration::from_secs(120), "{worst:?}");
        assert!(STARTUP_RETRY_DELAYS
            .windows(2)
            .all(|pair| pair[0] <= pair[1]));
    }

    #[tokio::test]
    async fn test_startup_retry_succeeds_after_transient_failures() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let delays = [Duration::from_millis(1); 4];
        let result: Result<&str, String> = retry_startup(&delays, || {
            let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if call < 2 {
                    Err(format!("第 {call} 次连不上"))
                } else {
                    Ok("连上了")
                }
            }
        })
        .await;
        assert_eq!(result, Ok("连上了"));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_startup_retry_gives_up_with_the_last_error() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let delays = [Duration::from_millis(1); 2];
        let result: Result<(), String> = retry_startup(&delays, || {
            let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move { Err(format!("第 {call} 次连不上")) }
        })
        .await;
        assert_eq!(result, Err("第 2 次连不上".to_string()));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    /// 模拟回滚: 库里已有一条本二进制没有的 (更新的) 迁移, 再跑一遍迁移不能失败.
    #[tokio::test]
    #[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
    async fn test_migrations_survive_rollback_to_older_binary() {
        let url = std::env::var("DATABASE_URL").expect("请设置独立测试库 DATABASE_URL");
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("migrate_test_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let session_schema = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .after_connect(move |connection, _| {
                let schema = session_schema.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('search_path', $1, false)")
                        .bind(schema)
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        migrator().run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
             VALUES (99999999, 'future', TRUE, '\\x00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let rerun = migrator().run(&pool).await;
        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
        rerun.unwrap();
    }

    #[test]
    fn user_storage_rejects_invalid_shapes() {
        let invalid = UserStorage {
            coal_prefs: json!([]),
            ..UserStorage::default()
        };
        assert_eq!(invalid.validate(), Err("coal_prefs 必须是 JSON 对象"));
    }

    #[test]
    fn measured_quality_requires_valid_value() {
        let empty = MeasuredQuality {
            s: None,
            a: None,
            v: None,
            g: None,
            y: None,
            m: None,
            csr: None,
            cri: None,
            m40: None,
            m10: None,
            bulk_density: None,
            coking_hours: None,
            flue_temp: None,
        };
        assert!(empty.validate().is_err());

        let valid = MeasuredQuality {
            csr: Some(62.0),
            ..empty.clone()
        };
        assert!(valid.validate().is_ok());

        // 炼焦条件各有量程, 不能套 0~100: 炉温 1350 合法, 装煤密度 2.0 不合法.
        let conditions = MeasuredQuality {
            bulk_density: Some(1.1),
            coking_hours: Some(25.0),
            flue_temp: Some(1350.0),
            ..empty.clone()
        };
        assert!(conditions.validate().is_ok());
        let too_dense = MeasuredQuality {
            bulk_density: Some(2.0),
            ..empty.clone()
        };
        assert_eq!(too_dense.validate(), Err("装煤密度应在 0.5~1.5 t/m³"));
        let coke_only = MeasuredQuality {
            cri: Some(25.0),
            m40: Some(88.0),
            m10: Some(6.5),
            ..empty
        };
        assert!(coke_only.validate().is_ok());
    }
}
