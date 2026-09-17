//! 煤库数据覆盖层: 校验 + 合并.
//!
//! 基线是 `blend_kit::master_json()` (编译期嵌入, 进 git, 可 review)。
//! 覆盖层存在 PostgreSQL `coal_overrides` 表, 由 `POST /api/master/coals` 写入。
//!
//! **只填空, 不覆盖**: 基线已有的指标一律拒绝写入。基线里的值是人工 review 过的
//! (往往是实测化验单), 而调研表推来的多是 Mysteel 规格界限 (≥/≤); 让界限值盖过实测值
//! 会让求解器以为配方合规。要改基线值, 走 git 提 PR, 保留人工审阅。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 8 项化验指标及其物理量程. 与 `scripts/check_master_data.mjs`、
/// `migrations/0002_coal_overrides.sql` 的 CHECK 保持一致 —— 三处都拦。
pub const INDICATOR_RANGE: &[(&str, f64, f64)] = &[
    ("S", 0.0, 10.0),
    ("A", 0.0, 50.0),
    ("V", 0.0, 50.0),
    ("G", 0.0, 100.0),
    ("Y", 0.0, 50.0),
    ("petro", 0.0, 1.0),
    ("CSR", 0.0, 100.0),
    ("M", 0.0, 30.0),
];

const CONFIDENCES: &[&str] = &["high", "medium", "low"];

/// 单批条数上限. 合法批次天然封顶: 基线 112 座煤 × 8 项 − 已填项 ≈ 220 条空缺,
/// 所以 300 不会误伤真实用法, 但挡住了"一次几万条"的放大 (每条非法项都会生成
/// 一条错误串塞进 422 响应)。与 import_history 的 100 条上限同型。
pub const MAX_UPDATES: usize = 300;
/// source 长度上限. 它会被写进 note, 而 note 每次 GET /api/master 都要回给前端,
/// 不封顶等于让主数据请求被永久撑大。
const MAX_SOURCE_CHARS: usize = 200;

/// 一条待写入的指标更新.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoalUpdate {
    /// 煤名, 必须已存在于基线 master 中 (本接口只补指标, 不新增煤源).
    pub coal: String,
    /// 指标名, 取 INDICATOR_RANGE 中之一.
    pub field: String,
    pub value: f64,
    /// 来源与原始口径, 例如 "2026-09-17 Mysteel 规格 Y≥16".
    /// 保留原始 ≥/≤ 符号, 以便日后区分界限值与实测值.
    pub source: String,
    #[serde(default = "default_confidence")]
    pub confidence: String,
}

fn default_confidence() -> String {
    "medium".to_string()
}

/// 已落库的一条覆盖.
#[derive(Clone, Debug, Serialize)]
pub struct StoredOverride {
    pub coal_name: String,
    pub field: String,
    pub value: f64,
    pub source: String,
    pub confidence: String,
    pub updated_at: String,
}

fn range_of(field: &str) -> Option<(f64, f64)> {
    INDICATOR_RANGE
        .iter()
        .find(|(name, _, _)| *name == field)
        .map(|(_, lo, hi)| (*lo, *hi))
}

/// 校验一批更新. 任一条不合法则整批拒绝 —— 半批生效会让调用方难以判断实际状态.
///
/// 返回每条不合法项的中文说明.
pub fn validate(updates: &[CoalUpdate]) -> Result<(), Vec<String>> {
    let master = match blend_kit::CoalMaster::load_embedded() {
        Ok(m) => m,
        Err(e) => return Err(vec![format!("基线 master 加载失败: {e}")]),
    };

    let mut errors = Vec::new();
    if updates.is_empty() {
        errors.push("updates 不能为空".to_string());
    }
    if updates.len() > MAX_UPDATES {
        // 提前返回: 否则下面会为超大批次逐条生成错误串, 正是要避免的放大.
        return Err(vec![format!(
            "一次最多提交 {MAX_UPDATES} 条, 实际 {}",
            updates.len()
        )]);
    }

    let mut seen = std::collections::HashSet::new();
    for (i, u) in updates.iter().enumerate() {
        let at = format!("第 {} 条 ({}/{})", i + 1, u.coal, u.field);

        if !seen.insert((u.coal.as_str(), u.field.as_str())) {
            errors.push(format!("{at}: 同一批次内重复"));
            continue;
        }

        let Some(entry) = master.find(&u.coal) else {
            errors.push(format!(
                "{at}: 基线 master 中没有这个煤名, 本接口不新增煤源"
            ));
            continue;
        };

        let Some((lo, hi)) = range_of(&u.field) else {
            let names: Vec<&str> = INDICATOR_RANGE.iter().map(|(n, _, _)| *n).collect();
            errors.push(format!("{at}: 未知指标, 只接受 {}", names.join("/")));
            continue;
        };

        if !u.value.is_finite() {
            errors.push(format!("{at}: 数值必须有限"));
            continue;
        }
        if u.value < lo || u.value > hi {
            errors.push(format!("{at}: {} 超出物理量程 [{lo}, {hi}]", u.value));
            continue;
        }

        if !CONFIDENCES.contains(&u.confidence.as_str()) {
            errors.push(format!("{at}: confidence 只接受 {}", CONFIDENCES.join("/")));
            continue;
        }

        if u.source.trim().is_empty() {
            errors.push(format!("{at}: source 不能为空, 需注明来源与口径"));
            continue;
        }
        if u.source.chars().count() > MAX_SOURCE_CHARS {
            errors.push(format!("{at}: source 不得超过 {MAX_SOURCE_CHARS} 字符"));
            continue;
        }

        // 只填空: 基线已有该指标则拒绝, 引导调用方走 git review.
        if entry.props.contains_key(&u.field) {
            let existing = entry.props.get(&u.field).copied().unwrap_or_default();
            errors.push(format!(
                "{at}: 基线已有该指标 (={existing}), 本接口只补空缺; \
                 要修正已有值请改 coal_master.json 并提 PR"
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// 把覆盖层叠加到基线 master 上, 返回合并后的 JSON 文本.
///
/// 只改动条目已有的 `props` / `confidence` / `note` 三个字段, 不引入新 schema,
/// 因此 `doudou_blend/src/types.ts` 无需同步改动。
pub fn merge(overrides: &[StoredOverride]) -> Result<String, String> {
    let mut doc: Value = serde_json::from_str(blend_kit::master_json())
        .map_err(|e| format!("基线 master 解析失败: {e}"))?;

    if overrides.is_empty() {
        return serde_json::to_string(&doc).map_err(|e| e.to_string());
    }

    let coals = doc
        .get_mut("coals")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "基线 master 缺少 coals 数组".to_string())?;

    for coal in coals.iter_mut() {
        let name = coal
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mine: Vec<&StoredOverride> = overrides.iter().filter(|o| o.coal_name == name).collect();
        if mine.is_empty() {
            continue;
        }

        // 该条目没有 props 对象 -> 跳过这一座煤, 而不是让整个 merge 失败.
        // 整体失败会让 master handler 退回纯基线, 于是**所有**覆盖一起静默消失,
        // 前端毫无感知 —— 一条脏基线不该拖垮整个覆盖层.
        if !coal.get("props").is_some_and(Value::is_object) {
            tracing::warn!(coal = %name, "基线条目缺 props 对象, 跳过其覆盖层");
            continue;
        }

        let mut applied = Vec::new();
        for o in &mine {
            let Some(props) = coal.get_mut("props").and_then(Value::as_object_mut) else {
                continue;
            };
            // 基线已有则基线优先 —— 写入时已拦, 这里是防御 (基线可能后来补上了该值).
            if props.contains_key(&o.field) {
                continue;
            }
            let number = serde_json::Number::from_f64(o.value)
                .ok_or_else(|| format!("{name}/{}: 数值无法序列化", o.field))?;
            props.insert(o.field.clone(), Value::Number(number));

            if !coal.get("confidence").is_some_and(|c| c.is_object()) {
                coal["confidence"] = Value::Object(serde_json::Map::new());
            }
            coal["confidence"][&o.field] = Value::String(o.confidence.clone());
            applied.push(o);
        }

        if applied.is_empty() {
            continue;
        }
        let provenance = applied
            .iter()
            .map(|o| format!("{}={} ({})", o.field, o.value, o.source))
            .collect::<Vec<_>>()
            .join("; ");
        let note = coal.get("note").and_then(Value::as_str).unwrap_or_default();
        let merged_note = if note.is_empty() {
            format!("[覆盖层] {provenance}")
        } else {
            format!("{note} | [覆盖层] {provenance}")
        };
        coal["note"] = Value::String(merged_note);
    }

    serde_json::to_string(&doc).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(coal: &str, field: &str, value: f64) -> CoalUpdate {
        CoalUpdate {
            coal: coal.into(),
            field: field.into(),
            value,
            source: "测试来源".into(),
            confidence: "medium".into(),
        }
    }

    /// 基线里确实缺 CSR 的煤可以补上.
    #[test]
    fn accepts_filling_a_gap() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| !c.props.is_empty() && !c.props.contains_key("CSR"))
            .expect("基线里应有缺 CSR 的煤");
        assert!(validate(&[update(&target.name, "CSR", 62.0)]).is_ok());
    }

    /// 基线已有的指标一律拒绝 —— 规格界限不得盖过实测值.
    #[test]
    fn rejects_overwriting_baseline() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| c.props.contains_key("S"))
            .expect("基线里应有带 S 的煤");
        let errors = validate(&[update(&target.name, "S", 1.0)]).unwrap_err();
        assert!(errors[0].contains("只补空缺"), "实际: {}", errors[0]);
    }

    /// 2026-09-17 那批"岩相=2026"必须被拦下.
    #[test]
    fn rejects_out_of_range_petro() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| !c.props.contains_key("petro"))
            .map(|c| c.name.clone())
            .unwrap_or_else(|| master.coals[0].name.clone());
        let errors = validate(&[update(&target, "petro", 2026.0)]).unwrap_err();
        assert!(
            errors[0].contains("超出物理量程") || errors[0].contains("只补空缺"),
            "实际: {}",
            errors[0]
        );
    }

    #[test]
    fn rejects_unknown_coal_and_field() {
        let e = validate(&[update("不存在的煤", "S", 1.0)]).unwrap_err();
        assert!(e[0].contains("没有这个煤名"));

        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let name = master.coals[0].name.clone();
        let e = validate(&[update(&name, "灰分", 1.0)]).unwrap_err();
        assert!(e[0].contains("未知指标"));
    }

    #[test]
    fn rejects_empty_batch_and_duplicates() {
        assert!(validate(&[]).is_err());
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| !c.props.is_empty() && !c.props.contains_key("CSR"))
            .unwrap();
        let errors = validate(&[
            update(&target.name, "CSR", 60.0),
            update(&target.name, "CSR", 65.0),
        ])
        .unwrap_err();
        assert!(errors[0].contains("重复"));
    }

    #[test]
    fn rejects_bad_confidence_and_blank_source() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| !c.props.is_empty() && !c.props.contains_key("CSR"))
            .unwrap();

        let mut bad = update(&target.name, "CSR", 60.0);
        bad.confidence = "很高".into();
        assert!(validate(&[bad]).unwrap_err()[0].contains("confidence"));

        let mut blank = update(&target.name, "CSR", 60.0);
        blank.source = "   ".into();
        assert!(validate(&[blank]).unwrap_err()[0].contains("source"));
    }

    /// 合并把值写进 props, 并把来源追加到 note.
    #[test]
    fn merge_applies_override_and_records_provenance() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| !c.props.is_empty() && !c.props.contains_key("CSR"))
            .unwrap();

        let merged = merge(&[StoredOverride {
            coal_name: target.name.clone(),
            field: "CSR".into(),
            value: 63.0,
            source: "2026-09-17 Mysteel 规格 CSR≥63".into(),
            confidence: "medium".into(),
            updated_at: "2026-09-17T00:00:00Z".into(),
        }])
        .unwrap();

        let doc: Value = serde_json::from_str(&merged).unwrap();
        let entry = doc["coals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == target.name.as_str())
            .unwrap();
        assert_eq!(entry["props"]["CSR"], 63.0);
        assert_eq!(entry["confidence"]["CSR"], "medium");
        assert!(entry["note"].as_str().unwrap().contains("[覆盖层]"));
    }

    /// 基线已有的值, 合并时基线优先 (防御: 基线后来补上了同一指标).
    #[test]
    fn merge_never_clobbers_baseline() {
        let master = blend_kit::CoalMaster::load_embedded().unwrap();
        let target = master
            .coals
            .iter()
            .find(|c| c.props.contains_key("S"))
            .unwrap();
        let baseline = target.props["S"];

        let merged = merge(&[StoredOverride {
            coal_name: target.name.clone(),
            field: "S".into(),
            value: baseline + 1.0,
            source: "应当被忽略".into(),
            confidence: "low".into(),
            updated_at: "2026-09-17T00:00:00Z".into(),
        }])
        .unwrap();

        let doc: Value = serde_json::from_str(&merged).unwrap();
        let entry = doc["coals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == target.name.as_str())
            .unwrap();
        assert_eq!(entry["props"]["S"], baseline);
        assert!(!entry["note"]
            .as_str()
            .unwrap_or_default()
            .contains("[覆盖层]"));
    }

    /// 空覆盖层时返回的就是基线本身.
    #[test]
    fn merge_without_overrides_is_baseline() {
        let merged: Value = serde_json::from_str(&merge(&[]).unwrap()).unwrap();
        let baseline: Value = serde_json::from_str(blend_kit::master_json()).unwrap();
        assert_eq!(merged, baseline);
    }
}
