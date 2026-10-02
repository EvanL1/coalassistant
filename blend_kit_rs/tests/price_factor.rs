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
            {"indicator": "S", "direction": "Upper", "base": 1.30, "premium_rate": 150, "discount_rate": 250,
             "clamp": [0.70, 100], "linear_range": [0.70, 1.60]},
            {"indicator": "A", "direction": "Upper", "base": 10.5, "premium_rate": 72, "discount_rate": 72,
             "linear_range": [8, 12]},
            {"indicator": "G", "direction": "Lower", "base": 85, "premium_rate": 5, "discount_rate": 5,
             "clamp": [40, 95]},
            {"indicator": "Y", "direction": "Lower", "base": 15, "premium_rate": 6, "discount_rate": 6},
            {"indicator": "CSR", "direction": "Lower", "base": 65, "premium_rate": 10, "discount_rate": 10,
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
    assert!(result["price_base"].is_null());
    assert_eq!(result["price_base_from_group"], false);
}

#[test]
fn test_price_factor_fills_coal_values() {
    let result = run(&request(1.5, Some(default_factor())));
    assert_eq!(result["ok"], true);
    assert_eq!(result["coal_values"].as_array().unwrap().len(), 4);
    assert_eq!(result["price_base_from_group"], true);
    assert!((result["price_base"].as_f64().unwrap() - 2166.65).abs() < 1e-6);
    for (name, expected) in [
        ("荣欣", -12.25),
        ("新星", 46.35),
        ("北沟", 34.55),
        ("浮精", -68.65),
    ] {
        assert!((gap(&result, name) - expected).abs() < 1e-6, "{name}");
    }
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
        for field in ["coal_values", "price_base", "price_base_from_group"] {
            a.as_object_mut().unwrap().remove(field);
            b.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(a, b);
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
    for field in ["coal_values", "price_base", "price_base_from_group"] {
        old.as_object_mut().unwrap().remove(field);
    }
    let parsed: BlendResult = serde_json::from_value(old).unwrap();
    assert!(parsed.coal_values.is_empty());
    assert!(parsed.price_base.is_none());
}
