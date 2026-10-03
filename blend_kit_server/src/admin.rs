//! 管理端: 全局设置 (煤阶交互 k)、k 标定、CSR 模型状态.
//!
//! 全部只认管理员会话 (见 `require_admin`). 纯计算放在 blend_kit (`fit_k` 等),
//! 这里只做 PG 读写与 JSON 拼装; 能脱离数据库的部分都写成纯函数单测.

use std::borrow::Cow;
use std::collections::HashMap;

use axum::{
    body::Body,
    extract::{Json, State},
    http::{HeaderMap, Request},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

use super::{
    bad_request, database_error, merged_master_json, require_admin, require_database,
    require_json_content_type, AppState,
};

/// 煤阶交互系数 k 在 app_settings 里的键.
const RANK_K_KEY: &str = "rank_interaction_k";
/// 设置请求体上限. 只有一个数字, 4KB 足够.
const MAX_SETTINGS_BODY: usize = 4 * 1024;
/// CSR 回归的 6 项自变量 (混合后指标), 与前端 backend.ts 的 deriveMixed 一致.
const MIXED_KEYS: [&str; 6] = ["S", "A", "V", "G", "Y", "M"];

// ---------------------------------------------------------------------------
// 设置
// ---------------------------------------------------------------------------

/// 启动时读取全局 k. 只返回 > 0 的有效值; 读库失败只记日志 —— 设置坏了不能拖垮启动.
pub(crate) async fn load_rank_interaction_k(pool: &PgPool) -> Option<f64> {
    match read_rank_k(pool).await {
        Ok(k) => k.filter(|k| k.is_finite() && *k > 0.0),
        Err(error) => {
            tracing::error!(%error, "读取煤阶交互设置失败, 本次不注入");
            None
        }
    }
}

/// 请求没自带 `rank_interaction` (缺键或为 null) 且 k > 0 时注入 `{"k": k}`.
/// 其余情况 (没设置、请求自带、不是 JSON 对象) 原样返回, 保证字节不变.
pub(crate) fn inject_rank_interaction(body: &str, k: Option<f64>) -> Cow<'_, str> {
    let Some(k) = k.filter(|k| k.is_finite() && *k > 0.0) else {
        return Cow::Borrowed(body);
    };
    let Ok(Value::Object(mut request)) = serde_json::from_str::<Value>(body) else {
        // 坏 JSON 交给 solve_json 报错, 这里不替它判.
        return Cow::Borrowed(body);
    };
    if request
        .get("rank_interaction")
        .is_some_and(|v| !v.is_null())
    {
        return Cow::Borrowed(body);
    }
    request.insert("rank_interaction".into(), json!({ "k": k }));
    Cow::Owned(Value::Object(request).to_string())
}

/// 校验 PUT /api/admin/settings 请求体. 返回要保存的 k, None (null 或 0) 表示关闭.
fn parse_settings(body: &[u8]) -> Result<Option<f64>, &'static str> {
    let value: Value = serde_json::from_slice(body).map_err(|_| "请求体不是合法 JSON")?;
    let k = value
        .as_object()
        .ok_or("请求体须为 JSON 对象")?
        .get(RANK_K_KEY)
        .ok_or("缺少 rank_interaction_k")?;
    if k.is_null() {
        return Ok(None);
    }
    let k = k.as_f64().ok_or("rank_interaction_k 须为数字或 null")?;
    if !k.is_finite() || k < 0.0 {
        return Err("rank_interaction_k 须为非负有限数");
    }
    Ok((k > 0.0).then_some(k))
}

fn settings_response(k: Option<f64>) -> Response {
    Json(json!({ "ok": true, "settings": { RANK_K_KEY: k } })).into_response()
}

/// GET /api/admin/settings
pub(crate) async fn get_settings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match read_rank_k(pool).await {
        Ok(k) => settings_response(k),
        Err(error) => database_error(error),
    }
}

/// PUT /api/admin/settings —— 收 `Request<Body>`, 先鉴权再解析 (理由见 update_coal_data).
pub(crate) async fn put_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    if let Err(response) = require_json_content_type(&headers) {
        return *response;
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_SETTINGS_BODY).await else {
        return bad_request("请求体读取失败或超出大小上限");
    };
    let k = match parse_settings(&body) {
        Ok(k) => k,
        Err(reason) => return bad_request(reason),
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match write_rank_k(pool, k).await {
        Ok(()) => {
            // 落库成功后才更新缓存, 求解从缓存取 k.
            state.set_rank_k(k);
            tracing::info!(?k, "煤阶交互 k 已更新");
            settings_response(k)
        }
        Err(error) => database_error(error),
    }
}

async fn read_rank_k(pool: &PgPool) -> Result<Option<f64>, sqlx::Error> {
    let value: Option<Value> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = $1")
        .bind(RANK_K_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(value.and_then(|v| v.as_f64()))
}

/// 关闭 (None) 即删行, 库里不留 0 或 null.
async fn write_rank_k(pool: &PgPool, k: Option<f64>) -> Result<(), sqlx::Error> {
    match k {
        Some(k) => {
            sqlx::query(
                r#"
                INSERT INTO app_settings (key, value, updated_at)
                VALUES ($1, $2, NOW())
                ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()
                "#,
            )
            .bind(RANK_K_KEY)
            .bind(json!(k))
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM app_settings WHERE key = $1")
                .bind(RANK_K_KEY)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 历史实测 (k 标定与 CSR 模型共用)
// ---------------------------------------------------------------------------

/// 一条有实测 CSR 的历史方案.
struct MeasuredRow {
    id: String,
    occurred_at: DateTime<Utc>,
    csr_measured: f64,
    result: Value,
}

async fn measured_history(pool: &PgPool, username: &str) -> Result<Vec<MeasuredRow>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, occurred_at, csr_measured, result_json
        FROM web_blend_history
        WHERE username = $1 AND csr_measured IS NOT NULL AND result_json IS NOT NULL
        ORDER BY occurred_at
        "#,
    )
    .bind(username)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(MeasuredRow {
                id: row.try_get("id")?,
                occurred_at: row.try_get("occurred_at")?,
                csr_measured: row.try_get("csr_measured")?,
                result: row.try_get("result_json")?,
            })
        })
        .collect()
}

/// 结果里某项指标的混合值 (indicator_check[].value).
fn indicator_value(result: &Value, indicator: &str) -> Option<f64> {
    result
        .get("indicator_check")?
        .as_array()?
        .iter()
        .find(|check| check.get("indicator").and_then(Value::as_str) == Some(indicator))?
        .get("value")?
        .as_f64()
}

// ---------------------------------------------------------------------------
// k 标定
// ---------------------------------------------------------------------------

/// 当前煤库 (基线 + 覆盖层) 每种煤的煤阶. 解析失败时返回空表 (所有样本都会被跳过).
fn master_ranks(master_json: &str) -> HashMap<String, Option<f64>> {
    let master: blend_kit::CoalMaster = match serde_json::from_str(master_json) {
        Ok(master) => master,
        Err(error) => {
            tracing::error!(%error, "煤库解析失败, 无法计算煤阶");
            return HashMap::new();
        }
    };
    master
        .coals
        .into_iter()
        .map(|entry| {
            let coal = blend_kit::Coal {
                name: entry.name.clone(),
                props: entry.props,
                fob: 0.0,
                frt: 0.0,
                petrography: None,
                purchase_terms: None,
            };
            (entry.name, blend_kit::coal_rank(&coal))
        })
        .collect()
}

/// 一条历史结果的 (不计交互的线性 CSR, 煤阶方差 D). 不能用时返回跳过理由.
fn calibration_point(
    result: &Value,
    ranks: &HashMap<String, Option<f64>>,
) -> Result<(f64, f64), String> {
    let csr = indicator_value(result, "CSR").ok_or("体检里没有 CSR")?;
    // 存的 CSR 已扣过罚项, 加回来才是线性值.
    let penalty = result
        .get("csr_interaction_penalty")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let recipe = result
        .get("recipe")
        .and_then(Value::as_object)
        .ok_or("结果里没有配方")?;
    let mut parts = Vec::with_capacity(recipe.len());
    for (name, ratio) in recipe {
        let ratio = ratio
            .as_f64()
            .ok_or_else(|| format!("{name} 配比不是数字"))?;
        if ratio <= 0.0 {
            continue;
        }
        let rank = match ranks.get(name) {
            None => return Err(format!("煤库里找不到 {name}")),
            Some(None) => return Err(format!("{name} 缺挥发和煤岩数据, 算不出煤阶")),
            Some(Some(rank)) => *rank,
        };
        parts.push((ratio, rank));
    }
    if parts.is_empty() {
        return Err("配方为空".into());
    }
    Ok((csr + penalty, blend_kit::rank_moments(&parts).1))
}

/// 由历史实测拟合 k, 拼成接口响应里的 calibration 对象.
fn calibrate(rows: &[MeasuredRow], ranks: &HashMap<String, Option<f64>>) -> Value {
    let mut points = Vec::new();
    let mut skipped = Vec::new();
    let mut samples = Vec::new();
    for row in rows {
        match calibration_point(&row.result, ranks) {
            Ok((base_csr, d)) => {
                samples.push((d, base_csr - row.csr_measured));
                points.push(json!({
                    "id": row.id,
                    "occurred_at": row.occurred_at.to_rfc3339(),
                    "base_csr": base_csr,
                    "measured_csr": row.csr_measured,
                    "rank_variance": d,
                }));
            }
            Err(reason) => skipped.push(json!({ "id": row.id, "reason": reason })),
        }
    }
    let fit = blend_kit::fit_k(&samples);
    let (recommended, reason) = fit.recommendation();
    json!({
        "n": fit.n,
        "skipped": skipped.len(),
        "skipped_details": skipped,
        "k": fit.k,
        "intercept": fit.intercept,
        "k_std_error": fit.k_std_error,
        "d_min": fit.d_min,
        "d_max": fit.d_max,
        "recommended": recommended,
        "reason": reason,
        "points": points,
    })
}

/// GET /api/admin/calibration —— 用普通账号的历史实测反推煤阶交互 k.
pub(crate) async fn calibration(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    let rows = match measured_history(pool, &state.auth.username).await {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let ranks = master_ranks(&merged_master_json(&state).await);
    Json(json!({ "ok": true, "calibration": calibrate(&rows, &ranks) })).into_response()
}

// ---------------------------------------------------------------------------
// CSR 模型状态
// ---------------------------------------------------------------------------

/// 历史结果的 6 项混合指标 + 实测 CSR. 缺任一项返回 None (同前端 deriveMixed).
fn csr_observation(result: &Value, csr_measured: f64) -> Option<blend_kit::CsrObservation> {
    let [s, a, v, g, y, m] = MIXED_KEYS.map(|key| indicator_value(result, key));
    Some(blend_kit::CsrObservation {
        s: s?,
        a: a?,
        v: v?,
        g: g?,
        y: y?,
        m: m?,
        csr_measured,
    })
}

fn csr_model_status(observations: &[blend_kit::CsrObservation]) -> Value {
    let required = blend_kit::ModelPolicy::default().min_csr_samples;
    let ready = observations.len() >= required;
    let r_squared = ready
        .then(|| blend_kit::CsrPredictor::fit(observations).ok())
        .flatten()
        .map(|predictor| predictor.r_squared)
        .filter(|r| r.is_finite());
    json!({
        "samples": observations.len(),
        "required": required,
        "ready": ready,
        "r_squared": r_squared,
    })
}

/// GET /api/admin/csr-model —— CSR 回归的样本进度与拟合优度.
pub(crate) async fn csr_model(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_admin(&headers, &state) {
        return *response;
    }
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    let rows = match measured_history(pool, &state.auth.username).await {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };
    let observations: Vec<_> = rows
        .iter()
        .filter_map(|row| csr_observation(&row.result, row.csr_measured))
        .collect();
    Json(json!({ "ok": true, "model": csr_model_status(&observations) })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_without_k_keeps_body_bytes() {
        let body = r#"{"coals":[],  "specs":[]}"#;
        assert!(matches!(inject_rank_interaction(body, None), Cow::Borrowed(b) if b == body));
        assert!(matches!(
            inject_rank_interaction(body, Some(0.0)),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            inject_rank_interaction("{not json", Some(50.0)),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            inject_rank_interaction("[1]", Some(50.0)),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn test_inject_adds_k_when_absent_or_null() {
        for body in [r#"{"coals":[]}"#, r#"{"coals":[],"rank_interaction":null}"#] {
            let injected: Value =
                serde_json::from_str(&inject_rank_interaction(body, Some(66.0))).unwrap();
            assert_eq!(injected["rank_interaction"], json!({ "k": 66.0 }));
            assert_eq!(injected["coals"], json!([]));
        }
    }

    #[test]
    fn test_inject_respects_request_setting() {
        let body = r#"{"rank_interaction":{"k":10}}"#;
        assert_eq!(inject_rank_interaction(body, Some(66.0)), body);
    }

    #[test]
    fn test_parse_settings_validation() {
        assert_eq!(
            parse_settings(br#"{"rank_interaction_k":66.5}"#),
            Ok(Some(66.5))
        );
        assert_eq!(parse_settings(br#"{"rank_interaction_k":null}"#), Ok(None));
        assert_eq!(parse_settings(br#"{"rank_interaction_k":0}"#), Ok(None));
        for bad in [
            &br#"{"rank_interaction_k":-1}"#[..],
            br#"{"rank_interaction_k":"66"}"#,
            br#"{}"#,
            br#"[]"#,
            b"{not json",
            br#"{"rank_interaction_k":1e400}"#,
        ] {
            assert!(
                parse_settings(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    fn fixture_result(recipe: Value, csr: f64, penalty: Option<f64>) -> Value {
        let mut result = json!({
            "recipe": recipe,
            "indicator_check": [
                { "indicator": "S", "value": 0.6 },
                { "indicator": "A", "value": 9.5 },
                { "indicator": "V", "value": 26.0 },
                { "indicator": "G", "value": 78.0 },
                { "indicator": "Y", "value": 16.0 },
                { "indicator": "M", "value": 9.0 },
                { "indicator": "CSR", "value": csr },
            ],
        });
        if let Some(penalty) = penalty {
            result["csr_interaction_penalty"] = json!(penalty);
        }
        result
    }

    fn fixture_ranks() -> HashMap<String, Option<f64>> {
        HashMap::from([
            ("甲".to_string(), Some(1.0)),
            ("乙".to_string(), Some(1.5)),
            ("无阶".to_string(), None),
        ])
    }

    #[test]
    fn test_calibration_point_adds_back_penalty_and_normalizes() {
        let result = fixture_result(
            json!({ "甲": 60.0, "乙": 40.0, "零": 0.0 }),
            62.0,
            Some(3.0),
        );
        let (base, d) = calibration_point(&result, &fixture_ranks()).unwrap();
        assert!((base - 65.0).abs() < 1e-12);
        assert!((d - 0.06).abs() < 1e-12);
    }

    #[test]
    fn test_calibration_point_skip_reasons() {
        let ranks = fixture_ranks();
        let unknown = fixture_result(json!({ "丙": 1.0 }), 60.0, None);
        assert!(calibration_point(&unknown, &ranks)
            .unwrap_err()
            .contains("找不到"));
        let no_rank = fixture_result(json!({ "无阶": 1.0 }), 60.0, None);
        assert!(calibration_point(&no_rank, &ranks)
            .unwrap_err()
            .contains("煤阶"));
        let no_csr = json!({ "recipe": { "甲": 1.0 }, "indicator_check": [] });
        assert!(calibration_point(&no_csr, &ranks)
            .unwrap_err()
            .contains("CSR"));
    }

    #[test]
    fn test_calibrate_fits_and_counts_skipped() {
        let ranks = fixture_ranks();
        let row = |id: &str, x: f64, measured: f64| {
            let result = fixture_result(json!({ "甲": x, "乙": 1.0 - x }), 70.0, None);
            MeasuredRow {
                id: id.into(),
                occurred_at: DateTime::<Utc>::UNIX_EPOCH,
                csr_measured: measured,
                result,
            }
        };
        // D = 0.25·x(1−x); 让残差恰为 1 + 40·D
        let mut rows: Vec<MeasuredRow> = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let d = 0.25 * x * (1.0 - x);
                row(&format!("h{i}"), *x, 70.0 - (1.0 + 40.0 * d))
            })
            .collect();
        rows.push(MeasuredRow {
            id: "bad".into(),
            occurred_at: DateTime::<Utc>::UNIX_EPOCH,
            csr_measured: 60.0,
            result: fixture_result(json!({ "丙": 1.0 }), 60.0, None),
        });

        let out = calibrate(&rows, &ranks);
        assert_eq!(out["n"], 9);
        assert_eq!(out["skipped"], 1);
        assert_eq!(out["skipped_details"][0]["id"], "bad");
        assert!((out["k"].as_f64().unwrap() - 40.0).abs() < 1e-6);
        assert!((out["intercept"].as_f64().unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(out["recommended"], true);
        assert_eq!(out["points"].as_array().unwrap().len(), 9);
        assert_eq!(out["points"][0]["base_csr"], 70.0);
    }

    #[test]
    fn test_master_ranks_uses_vdaf_fallback() {
        let master = json!({
            "version": "t", "updated_at": "t", "description": "t",
            "default_contract": { "name": "t", "specs": [] },
            "coals": [
                { "name": "有挥发", "status": "verified", "props": { "V": 25.0 } },
                { "name": "无挥发", "status": "verified", "props": {} },
            ],
        });
        let ranks = master_ranks(&master.to_string());
        assert!(ranks["有挥发"].is_some());
        assert_eq!(ranks["无挥发"], None);
        assert!(master_ranks("{bad").is_empty());
    }

    #[test]
    fn test_csr_observation_needs_all_six() {
        let full = fixture_result(json!({}), 60.0, None);
        let obs = csr_observation(&full, 68.0).unwrap();
        assert_eq!((obs.s, obs.m, obs.csr_measured), (0.6, 9.0, 68.0));
        let partial = json!({ "indicator_check": [{ "indicator": "S", "value": 0.6 }] });
        assert!(csr_observation(&partial, 68.0).is_none());
    }

    #[test]
    fn test_csr_model_status_not_ready() {
        let status = csr_model_status(&[]);
        assert_eq!(status["samples"], 0);
        assert_eq!(status["ready"], false);
        assert!(status["r_squared"].is_null());
        assert_eq!(
            status["required"],
            blend_kit::ModelPolicy::default().min_csr_samples
        );
    }
}
