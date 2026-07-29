use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

const DEFAULT_QUANTITY: f64 = 3700.0;

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
}

impl Default for UserStorage {
    fn default() -> Self {
        Self {
            initialized: false,
            coal_prefs: empty_object(),
            contract: None,
            quantity: DEFAULT_QUANTITY,
            user_coals: empty_array(),
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
}

impl MeasuredQuality {
    pub fn validate(&self) -> Result<(), &'static str> {
        let values = [self.s, self.a, self.v, self.g, self.y, self.m, self.csr];
        if values.iter().all(Option::is_none) {
            return Err("至少提供一项实测值");
        }
        if values
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || value <= 0.0 || value > 100.0)
        {
            return Err("实测值必须在 0 到 100 之间");
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
}

pub async fn connect_from_env() -> Option<PgPool> {
    let on_railway = std::env::var_os("RAILWAY_ENVIRONMENT_NAME").is_some();
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ if on_railway => panic!("Railway 部署必须设置 DATABASE_URL"),
        _ => return None,
    };

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .unwrap_or_else(|error| panic!("无法连接 PostgreSQL: {error}"));
    sqlx::migrate!()
        .run(&pool)
        .await
        .unwrap_or_else(|error| panic!("数据库迁移失败: {error}"));
    Some(pool)
}

pub async fn health(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

pub async fn load_storage(pool: &PgPool, username: &str) -> Result<UserStorage, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT coal_prefs, contract, quantity, user_coals
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
            username, coal_prefs, contract, quantity, user_coals, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, NOW())
        ON CONFLICT (username) DO UPDATE SET
            coal_prefs = EXCLUDED.coal_prefs,
            contract = EXCLUDED.contract,
            quantity = EXCLUDED.quantity,
            user_coals = EXCLUDED.user_coals,
            updated_at = NOW()
        "#,
    )
    .bind(username)
    .bind(&storage.coal_prefs)
    .bind(&storage.contract)
    .bind(storage.quantity)
    .bind(&storage.user_coals)
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
            y_measured, m_measured
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
            csr_measured = COALESCE($9, csr_measured)
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
        };
        assert!(empty.validate().is_err());

        let valid = MeasuredQuality {
            csr: Some(62.0),
            ..empty
        };
        assert!(valid.validate().is_ok());
    }
}
