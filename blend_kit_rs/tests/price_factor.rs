//! 价格因子 (`BlendRequest.price_factor`) 的求解级行为: 只评估, 不进 LP.
//!
//! 全部走公开 JSON 入口 `solve_json`, 数据都是 fixture, 不读 master.

use blend_kit::*;
use serde_json::{json, Value};

/// 4 种煤, fob 为出厂价, frt 0.
fn coals() -> Value {
    let coal = |name: &str, s: f64, a: f64, v: f64, g: f64, y: f64, csr: f64, fob: f64| {
        json!({
            "name": name, "fob": fob, "frt": 0.0,
            "props": {"S": s, "A": a, "V": v, "G": g, "Y": y, "petro": 0.1, "CSR": csr, "M": 10.0}
        })
    };
    json!([
        coal("荣欣", 0.4, 9.7, 24.0, 97.0, 23.0, 75.0, 2500.0),
        coal("新星", 1.6, 10.5, 20.0, 75.0, 17.0, 65.0, 2100.0),
        coal("北沟", 1.8, 8.6, 24.0, 94.0, 22.0, 55.0, 2200.0),
        coal("浮精", 0.8, 12.0, 24.0, 80.0, 20.0, 58.0, 2000.0),
    ])
}

fn default_factor() -> Value {
    json!({
        "adjusts": [
            {"indicator": "S", "direction": "Upper", "base": 1.30, "premium_rate": 75, "discount_rate": 125,
             "clamp": [0.70, 100], "linear_range": [0.70, 1.60]},
            {"indicator": "A", "direction": "Upper", "base": 10.5, "premium_rate": 36, "discount_rate": 36,
             "linear_range": [8, 12]},
            {"indicator": "G", "direction": "Lower", "base": 85, "premium_rate": 2.5, "discount_rate": 2.5,
             "clamp": [40, 95]},
            {"indicator": "Y", "direction": "Lower", "base": 15, "premium_rate": 3, "discount_rate": 3},
            {"indicator": "CSR", "direction": "Lower", "base": 65, "premium_rate": 5, "discount_rate": 5,
             "enabled": true}
        ]
    })
}

fn request(s_max: f64, factor: Option<Value>) -> Value {
    let mut req = json!({
        "coals": coals(),
        "specs": [
            {"indicator": "S", "direction": "Upper", "max": s_max, "enabled": true},
            {"indicator": "G", "direction": "Lower", "min": 85.0, "enabled": true}
        ],
        "total_quantity": 1000.0,
        "truncate_decimal": false
    });
    if let Some(factor) = factor {
        req["price_factor"] = factor;
    }
    req
}

fn run(req: &Value) -> Value {
    serde_json::from_str(&solve_json(&req.to_string())).unwrap()
}

fn gap(result: &Value, name: &str) -> f64 {
    result["coal_values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("coal_values 里没有 {name}"))["gap"]
        .as_f64()
        .unwrap()
}

#[test]
fn test_without_price_factor_coal_values_empty() {
    let result = run(&request(1.5, None));
    assert_eq!(result["ok"], true);
    assert_eq!(result["coal_values"], json!([]));
    assert!(result["market_price"].is_null());
    assert_eq!(result["market_price_from_group"], false);
}

#[test]
fn test_price_factor_fills_coal_values() {
    // 指定市场价 2000: 应值 = 2000/1000 × 质量指数 (1172.8 / 943.5 / 999.4 / 951).
    let mut factor = default_factor();
    factor["market_price"] = json!(2000.0);
    let result = run(&request(1.5, Some(factor)));
    assert_eq!(result["ok"], true);
    assert_eq!(result["coal_values"].as_array().unwrap().len(), 4);
    assert_eq!(result["market_price_from_group"], false);
    assert!((result["market_price"].as_f64().unwrap() - 2000.0).abs() < 1e-9);
    for (name, expected) in [
        ("荣欣", 2500.0 - 2345.6),
        ("新星", 2100.0 - 1887.0),
        ("北沟", 2200.0 - 1998.8),
        ("浮精", 2000.0 - 1902.0),
    ] {
        assert!((gap(&result, name) - expected).abs() < 1e-6, "{name}");
    }
}

/// 不给市场价时由煤池反推, 结果里要标明来源.
#[test]
fn test_market_price_inferred_when_absent() {
    let result = run(&request(1.5, Some(default_factor())));
    assert_eq!(result["market_price_from_group"], true);
    assert!(result["market_price"].as_f64().unwrap() > 0.0);
}

/// 回归: 价格因子只评估, 配方/成本/指标/警告与不带时完全相同.
#[test]
fn test_price_factor_does_not_change_solution() {
    for fixed in [
        None,
        Some(json!({"荣欣": 1, "新星": 1, "北沟": 1, "浮精": 1})),
    ] {
        let mut plain = request(1.5, None);
        let mut priced = request(1.5, Some(default_factor()));
        if let Some(fixed) = fixed {
            plain["fixed_ratios"] = fixed.clone();
            priced["fixed_ratios"] = fixed;
        }
        let mut a = run(&plain);
        let mut b = run(&priced);
        assert!(!b["coal_values"].as_array().unwrap().is_empty());
        for field in ["coal_values", "market_price", "market_price_from_group"] {
            a.as_object_mut().unwrap().remove(field);
            b.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(a, b);
    }
}

/// extra_coals: 没报价、不进 LP 的煤也打分给估算价; 配方与不带时完全相同.
#[test]
fn test_extra_coals_scored_but_not_blended() {
    let mut factor = default_factor();
    factor["market_price"] = json!(2000.0);
    // 质量远好于池中所有煤; 若误进 LP (价格 0) 必然被选中.
    factor["extra_coals"] = json!([{
        "name": "无价好煤", "fob": 0.0, "frt": 0.0,
        "props": {"S": 0.5, "A": 7.0, "V": 24.0, "G": 95.0, "Y": 25.0, "petro": 0.1, "CSR": 75.0, "M": 9.0}
    }]);
    let plain = run(&request(1.5, None));
    let result = run(&request(1.5, Some(factor)));
    assert_eq!(result["recipe"], plain["recipe"]);
    let values = result["coal_values"].as_array().unwrap();
    assert_eq!(values.len(), 5);
    let extra = values.iter().find(|v| v["name"] == "无价好煤").unwrap();
    assert!(extra["fob"].is_null() && extra["gap"].is_null() && extra["ratio"].is_null());
    assert!(extra["points"].as_f64().unwrap() > 1000.0);
    let fair = extra["fair"].as_f64().unwrap();
    assert!((fair - 2.0 * extra["points"].as_f64().unwrap()).abs() < 1e-9);
}

fn ratio(result: &Value, name: &str) -> f64 {
    result["recipe"][name].as_f64().unwrap_or(0.0)
}

fn blend_value(result: &Value, name: &str) -> f64 {
    result["coal_values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("coal_values 里没有 {name}"))["blend_value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{name} 没有 blend_value"))
}

/// 配煤价值 = 盈亏平衡到厂价: 没入选的煤降到它以下就入选, 高于它仍不入选.
#[test]
fn test_blend_value_is_break_even_for_unselected_coal() {
    let base = request(1.5, Some(default_factor()));
    let result = run(&base);
    let names = ["荣欣", "新星", "北沟", "浮精"];
    let out = *names
        .iter()
        .find(|name| ratio(&result, name) < 1e-6)
        .expect("fixture 里应有没入选的煤");
    let value = blend_value(&result, out);
    let index = names.iter().position(|name| *name == out).unwrap();
    for (delta, should_enter) in [(-5.0, true), (5.0, false)] {
        let mut req = base.clone();
        req["coals"][index]["fob"] = json!(value + delta);
        let probe = run(&req);
        assert_eq!(
            ratio(&probe, out) > 1e-6,
            should_enter,
            "{out} 价 {} (配煤价值 {value:.2}): 配方 {}",
            value + delta,
            probe["recipe"]
        );
    }
}

/// 已入选的煤, 配煤价值就是它现在的到厂价 (再贵一点仍可能入选, 那要做敏感度分析, 不在此).
#[test]
fn test_blend_value_of_selected_coal_equals_its_price() {
    let result = run(&request(1.5, Some(default_factor())));
    let coals = coals();
    for coal in coals.as_array().unwrap() {
        let name = coal["name"].as_str().unwrap();
        if ratio(&result, name) > 1e-3 {
            let cif = coal["fob"].as_f64().unwrap() + coal["frt"].as_f64().unwrap();
            assert!((blend_value(&result, name) - cif).abs() < 0.5, "{name}");
        }
    }
}

/// 没报价的 extra 煤也有配煤价值, 且它就是盈亏平衡价: 按它减 5 元放进煤池就入选.
#[test]
fn test_extra_coal_blend_value_is_break_even() {
    let extra = json!({
        "name": "无价煤", "fob": 0.0, "frt": 0.0,
        "props": {"S": 1.0, "A": 9.0, "V": 24.0, "G": 90.0, "Y": 20.0, "petro": 0.1, "CSR": 66.0, "M": 10.0}
    });
    let mut factor = default_factor();
    factor["extra_coals"] = json!([extra.clone()]);
    let result = run(&request(1.5, Some(factor)));
    assert!(ratio(&result, "无价煤") == 0.0, "extra 煤不得进配方");
    let value = blend_value(&result, "无价煤");
    assert!(value > 0.0);
    for (delta, should_enter) in [(-5.0, true), (5.0, false)] {
        let mut req = request(1.5, None);
        let mut priced = extra.clone();
        priced["fob"] = json!(value + delta);
        req["coals"].as_array_mut().unwrap().push(priced);
        let probe = run(&req);
        assert_eq!(
            ratio(&probe, "无价煤") > 1e-6,
            should_enter,
            "价 {} (配煤价值 {value:.2}): 配方 {}",
            value + delta,
            probe["recipe"]
        );
    }
}

#[test]
fn test_invalid_price_factor_does_not_fail_solve() {
    let mut factor = default_factor();
    factor["adjusts"][0]["direction"] = json!("Range");
    let plain = run(&request(1.5, None));
    let result = run(&request(1.5, Some(factor)));
    assert_eq!(result["ok"], true);
    assert_eq!(result["recipe"], plain["recipe"]);
    assert_eq!(result["coal_values"], json!([]));
    let warnings = result["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("价格因子")),
        "{warnings:?}"
    );
}

#[test]
fn test_infeasible_result_still_has_coal_values() {
    let result = run(&request(0.1, Some(default_factor())));
    assert_eq!(result["ok"], false);
    assert_eq!(result["coal_values"].as_array().unwrap().len(), 4);
}

/// 价格因子上线前保存的结果 (无新字段) 仍能读回.
#[test]
fn test_old_result_without_price_fields_deserializes() {
    let mut old = run(&request(1.5, None));
    for field in ["coal_values", "market_price", "market_price_from_group"] {
        old.as_object_mut().unwrap().remove(field);
    }
    let parsed: BlendResult = serde_json::from_value(old).unwrap();
    assert!(parsed.coal_values.is_empty());
    assert!(parsed.market_price.is_none());
}
