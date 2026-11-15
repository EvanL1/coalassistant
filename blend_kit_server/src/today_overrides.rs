//! 按次覆盖: 咨询时每个客户的合同要求、候选煤、报价都不一样, 这些只改这一次的请求, 不写任何存储.
//! 由 [`super::build`] 调用, 规则见 [`super::TodayOptions`] 的 `specs` / `only` / `quotes`.
use serde_json::{json, Map, Value};

use super::{
    normalize, pick_block_reason, request_coal, unknown_name_line, valid_number, NameProblems,
    Quote, QuotedCoal, Readiness, ResolvedCoal, TodayOptions, INDICATORS,
};

/// 本次排除点到了候选煤以外的煤时给的原因.
pub(super) const NOT_PICKED: &str = "不在本次候选煤里";

/// 一项按次覆盖的合同要求.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum SpecOverride {
    /// 本次不约束.
    Off,
    /// 给了哪一端就约束哪一端.
    Bound { min: Option<f64>, max: Option<f64> },
}

/// 解析 `specs`: 指标 → `{min, max}` 或 null, 按 [`INDICATORS`] 的顺序返回. 有一项写得不对就
/// 整个拒绝, 说明里列出可用的指标 —— 不能把"没改成"当成"改了"去求解.
pub(super) fn parse_specs(
    given: Option<&Map<String, Value>>,
) -> Result<Vec<(&'static str, SpecOverride)>, String> {
    let Some(given) = given else {
        return Ok(Vec::new());
    };
    let mut errors: Vec<String> = given
        .keys()
        .filter(|key| !INDICATORS.contains(&key.as_str()))
        .map(|key| format!("「{key}」不是合同指标"))
        .collect();
    let mut overrides = Vec::new();
    for indicator in INDICATORS {
        match given.get(indicator).map(parse_spec) {
            Some(Ok(spec)) => overrides.push((indicator, spec)),
            Some(Err(reason)) => errors.push(format!("{indicator}: {reason}")),
            None => {}
        }
    }
    if errors.is_empty() {
        return Ok(overrides);
    }
    let known: Vec<String> = INDICATORS
        .iter()
        .map(|key| format!("{key} ({})", crate::mcp::label(key)))
        .collect();
    Err(format!(
        "本次合同要求 (specs) 写得不对, 没有求解:\n- {}\n可用的指标: {}。\
         每项给 max (上限)、min (下限) 或两个都给; 写 null 表示本次不约束这一项.",
        errors.join("\n- "),
        known.join("、")
    ))
}

fn parse_spec(value: &Value) -> Result<SpecOverride, String> {
    if value.is_null() {
        return Ok(SpecOverride::Off);
    }
    let Some(object) = value.as_object() else {
        return Err("要么给 min / max, 要么写 null (本次不约束)".into());
    };
    let bound = |side: &str| match object.get(side).filter(|value| !value.is_null()) {
        None => Ok(None),
        Some(value) => valid_number(value)
            .map(Some)
            .ok_or_else(|| format!("{side} 须为不小于 0 的数")),
    };
    match (bound("min")?, bound("max")?) {
        (None, None) => Err("min 和 max 至少给一个; 本次不约束这一项就写 null".into()),
        (Some(min), Some(max)) if min > max => Err(format!("min {min} 大于 max {max}")),
        (min, max) => Ok(SpecOverride::Bound { min, max }),
    }
}

/// 在保存的合同上逐项覆盖, 返回 (本次的合同, 改过的指标, 要告诉调用方的说明). 没提到的指标照旧.
pub(super) fn apply_specs(
    specs: Value,
    overrides: &[(&'static str, SpecOverride)],
) -> (Value, Vec<String>, Vec<String>) {
    if overrides.is_empty() {
        return (specs, Vec::new(), Vec::new());
    }
    let mut list = match specs {
        Value::Array(list) => list,
        _ => Vec::new(),
    };
    let mut notes = Vec::new();
    for (indicator, spec) in overrides {
        let at: Vec<usize> = (0..list.len())
            .filter(|index| list[*index]["indicator"] == *indicator)
            .collect();
        // 同一指标存了几条 (不该有): 只留一条生效, 免得旧的那条还在暗中约束.
        let (rebind, disable) = match spec {
            SpecOverride::Off => (None, &at[..]),
            SpecOverride::Bound { min, max } => (Some((*min, *max)), at.get(1..).unwrap_or(&[])),
        };
        for index in disable {
            list[*index]["enabled"] = json!(false);
        }
        let Some((min, max)) = rebind else { continue };
        match at.first() {
            Some(first) => notes.extend(rebound(&mut list[*first], indicator, min, max)),
            None => {
                let mut fresh = Map::new();
                fresh.insert("indicator".into(), json!(indicator));
                set_bounds(&mut fresh, min, max);
                list.push(Value::Object(fresh));
            }
        }
    }
    let changed = overrides.iter().map(|(key, _)| key.to_string()).collect();
    (Value::Array(list), changed, notes)
}

/// 把保存的一条合同项改成本次的界限: 判定规则与安全余量照旧, 其余按硬约束算.
/// 原来不是硬约束的, 返回一句说明.
fn rebound(
    spec: &mut Value,
    indicator: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Option<String> {
    let label = crate::mcp::label(indicator);
    let note = match spec["enforcement"].as_str() {
        Some("Priced") => Some(format!("{label} 本次按硬约束算, 未计扣款条款")),
        Some("Soft" | "Advisory") => {
            Some(format!("{label} 本次按硬约束算 (保存的合同里它不是硬约束)"))
        }
        _ => None,
    };
    let object = spec.as_object_mut()?;
    object.remove("enforcement");
    object.remove("penalty");
    set_bounds(object, min, max);
    note
}

/// 只给上限是 Upper, 只给下限是 Lower, 两个都给是 Range; 用不到的那一端去掉.
fn set_bounds(spec: &mut Map<String, Value>, min: Option<f64>, max: Option<f64>) {
    let direction = match (min, max) {
        (Some(_), Some(_)) => "Range",
        (Some(_), None) => "Lower",
        _ => "Upper",
    };
    spec.insert("direction".into(), json!(direction));
    for (side, bound) in [("min", min), ("max", max)] {
        match bound {
            Some(value) => spec.insert(side.into(), json!(value)),
            None => spec.remove(side),
        };
    }
    spec.insert("enabled".into(), json!(true));
}

/// 本次按次点名的候选煤 (`only`, 或询价精算里报了价的煤): 归一化煤名, 按点名顺序去重;
/// 两样都没给为 None. 对不上的、已停用/重名/数据非法的记进 `problems`.
pub(super) fn pick_candidates(
    pool: &[ResolvedCoal],
    options: &TodayOptions,
    problems: &mut NameProblems,
) -> Option<Vec<String>> {
    let names: Vec<&str> = match (&options.quotes, &options.only) {
        (Some(quotes), _) => quotes.iter().map(|quote| quote.name.as_str()).collect(),
        (None, Some(only)) => only.iter().map(String::as_str).collect(),
        (None, None) => return None,
    };
    let mut keys = Vec::new();
    // 用不了的煤名重复点到时只说明一次.
    let mut refused: Vec<String> = Vec::new();
    for name in names {
        let key = normalize(name);
        let same: Vec<&ResolvedCoal> = pool
            .iter()
            .filter(|coal| normalize(&coal.name) == key)
            .collect();
        let unknown = key.is_empty() || same.is_empty();
        let refusal = if key.is_empty() {
            Some("点名的煤里有一项煤名是空的".to_string())
        } else if same.is_empty() {
            Some(unknown_name_line(pool, name, false))
        } else {
            pick_block_reason(same[0]).map(|reason| blocked_line(name, &same, reason))
        };
        if let Some(line) = refusal {
            problems.unknown |= unknown;
            problems.blocked |= !unknown;
            if !refused.contains(&key) {
                problems.lines.push(line);
                refused.push(key);
            }
        } else if !keys.contains(&key) {
            keys.push(key);
        } else if options.quotes.is_some() {
            // 候选煤重复点名无害; 同一种煤报了两个价就不知道该用哪个.
            problems.lines.push(format!(
                "报价里「{name}」和另一项是同一种煤「{}」, 请合并成一项",
                same[0].name
            ));
        }
    }
    Some(keys)
}

fn blocked_line(name: &str, same: &[&ResolvedCoal], reason: &str) -> String {
    if same[0].readiness == Readiness::DuplicateName {
        let names: Vec<&str> = same.iter().map(|coal| coal.name.as_str()).collect();
        format!("「{name}」这次用不了: {reason} ({})", names.join("、"))
    } else {
        format!("「{name}」这次用不了: {reason}")
    }
}

/// `only` 点到的煤在煤池里的写法, 按点名顺序, 去掉本次排除的.
pub(super) fn candidate_names(
    pool: &[ResolvedCoal],
    keys: &[String],
    excluded: &[String],
) -> Vec<String> {
    keys.iter()
        .filter(|key| !excluded.contains(key))
        .filter_map(|key| pool.iter().find(|coal| &normalize(&coal.name) == key))
        .map(|coal| coal.name.clone())
        .collect()
}

/// 按次点名后候选煤太少: 验算至少 1 种, 求最优与询价精算至少 2 种. 没点名不管 (照旧).
pub(super) fn too_few_candidates(options: &TodayOptions, count: usize) -> Option<String> {
    if options.quotes.is_some() {
        return (count < 2).then(|| format!("询价精算至少要两种煤的报价, 这次只有 {count} 种"));
    }
    options.only.as_ref()?;
    if options.fixed_ratios.is_some() {
        return (count == 0).then(|| "本次候选煤去掉排除的以后一种不剩, 没法验算".into());
    }
    (count < 2).then(|| {
        format!(
            "本次候选煤去掉排除的以后只剩 {count} 种, 求最低成本配方至少要 2 种; \
             请在 only 里多列几种煤"
        )
    })
}

/// 给了候选煤, 验算配比却点到候选煤以外的煤.
pub(super) fn outside_candidates(name: &str, problems: &mut NameProblems) {
    problems
        .lines
        .push(format!("「{name}」不在本次候选煤 (only) 里"));
    problems.outside = true;
}

/// 询价精算: 报了价的煤按实价进 LP, 不再按估算价并入; 价格因子照带, 核心据此给出报价与估价
/// 的差. 返回进了请求的报价; 不是询价精算为 None, 请求不动.
pub(super) fn price_by_quotes(
    pool: &[ResolvedCoal],
    options: &TodayOptions,
    request: &mut Value,
    factor: &mut Map<String, Value>,
    notes: &mut Vec<String>,
) -> Option<Vec<QuotedCoal>> {
    let (coals, quoted) = quoted_coals(pool, options.quotes.as_ref()?);
    request["coals"] = Value::Array(coals);
    factor.insert("extra_coals".into(), json!([]));
    factor.insert("estimate_unpriced".into(), json!(false));
    notes.extend(freight_note(&quoted));
    Some(quoted)
}

/// 询价精算进 LP 的煤: 煤池条目带上报价, 指标 = 煤池值 ∪ 当场改的值. 与返回的报价一一对应.
fn quoted_coals(pool: &[ResolvedCoal], quotes: &[Quote]) -> (Vec<Value>, Vec<QuotedCoal>) {
    let mut coals = Vec::new();
    let mut quoted = Vec::new();
    for quote in quotes {
        let key = normalize(&quote.name);
        let Some(coal) = pool.iter().find(|coal| normalize(&coal.name) == key) else {
            continue;
        };
        let mut entry = request_coal(coal);
        let mut changed = Vec::new();
        for indicator in INDICATORS {
            let Some(value) = quote.props.get(indicator).and_then(valid_number) else {
                continue;
            };
            if coal.props.get(indicator).and_then(Value::as_f64) != Some(value) {
                changed.push(indicator.to_string());
            }
            entry["props"][indicator] = json!(value);
        }
        let frt = quote.frt.unwrap_or(0.0);
        entry["fob"] = json!(quote.fob);
        entry["frt"] = json!(frt);
        coals.push(entry);
        quoted.push(QuotedCoal {
            name: coal.name.clone(),
            fob: quote.fob,
            frt,
            frt_given: quote.frt.is_some(),
            changed,
        });
    }
    (coals, quoted)
}

/// 没给运费的煤按 0 计, 要说出来: 到厂价少算一截, 配方会偏向这几种煤.
fn freight_note(quoted: &[QuotedCoal]) -> Option<String> {
    let missing: Vec<&str> = quoted
        .iter()
        .filter(|quote| !quote.frt_given)
        .map(|quote| quote.name.as_str())
        .collect();
    (!missing.is_empty()).then(|| {
        format!(
            "{} 种煤没给运费, 按 0 计: {}",
            missing.len(),
            missing.join("、")
        )
    })
}

#[cfg(test)]
#[path = "today_overrides_tests.rs"]
mod tests;
