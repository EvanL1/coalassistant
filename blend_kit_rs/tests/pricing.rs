//! 估价模块 (`estimate_json`): 按同煤种、指标最接近的基准报价估价.
//!
//! 全部走公开 JSON 入口, 数据都是 fixture, 不读 master. 系数取 master 默认表的形状:
//! S 升 75 / 扣 125 (低于 0.7 按 0.7), A 36, G 2.5 (截在 40–95), Y 3.

use blend_kit::*;
use serde_json::{json, Value};

fn adjusts() -> Value {
    json!([
        {"indicator": "S", "direction": "Upper", "base": 1.30, "premium_rate": 75, "discount_rate": 125,
         "clamp": [0.70, 100]},
        {"indicator": "A", "direction": "Upper", "base": 10.5, "premium_rate": 36, "discount_rate": 36},
        {"indicator": "G", "direction": "Lower", "base": 85, "premium_rate": 2.5, "discount_rate": 2.5,
         "clamp": [40, 95]},
        {"indicator": "Y", "direction": "Lower", "base": 15, "premium_rate": 3, "discount_rate": 3}
    ])
}

/// 三条基准: 质量指数分别为 987.5 / 1099 / 745.5.
fn benchmarks() -> Value {
    json!([
        {"name": "晋中中硫主焦", "coal_type": "焦煤", "price": 2050.0, "date": "2026-10-10",
         "props": {"A": 10.5, "S": 1.3, "V": 25.0, "G": 80.0}},
        {"name": "安泽低硫主焦", "coal_type": "焦煤", "price": 2250.0, "date": "2026-10-10",
         "props": {"A": 9.0, "S": 0.5, "V": 20.0, "G": 85.0}},
        {"name": "蒲县单一肥煤", "coal_type": "肥煤", "price": 2145.0, "date": "2026-10-09",
         "props": {"A": 11.0, "S": 3.8, "V": 29.0, "G": 95.0, "Y": 32.0}}
    ])
}

fn coal(name: &str, coal_type: &str, props: Value) -> Value {
    json!({"name": name, "coal_type": coal_type, "props": props, "fob": 0.0, "frt": 0.0})
}

fn coals() -> Value {
    json!([
        // 质量指数 1101.8, 离安泽 (差 50.2 点) 比离晋中 (111.3 点) 近.
        coal(
            "低硫焦",
            "焦煤",
            json!({"S": 0.6, "A": 9.7, "G": 96.0, "Y": 16.0})
        ),
        // 799 点, 只有一条肥煤基准.
        coal(
            "高硫肥",
            "肥煤",
            json!({"S": 3.9, "A": 9.5, "G": 100.0, "Y": 36.0})
        ),
        // 1029 点, 没有气煤基准: 退回全部基准里最近的 (安泽, 差 70 点).
        coal("某气煤", "气煤", json!({"S": 0.8, "A": 9.0, "G": 60.0}))
    ])
}

fn estimate(input: &Value) -> Value {
    serde_json::from_str(&estimate_json(&input.to_string())).unwrap()
}

fn value<'a>(output: &'a Value, name: &str) -> &'a Value {
    output["coal_values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["name"] == name)
        .unwrap_or_else(|| panic!("{name} 不在估价结果里: {output}"))
}

fn assert_close(actual: &Value, expected: f64) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("不是数: {actual}"));
    assert!((actual - expected).abs() < 0.01, "{actual} ≠ {expected}");
}

#[test]
fn test_prices_against_nearest_same_type_benchmark() {
    let output = estimate(&json!({
        "coals": coals(),
        "price_factor": {"adjusts": adjusts(), "benchmarks": benchmarks()}
    }));
    assert_eq!(output["ok"], true, "{output}");

    // 估价 = 基准价 × 本煤质量指数 / 基准品质量指数.
    let low = value(&output, "低硫焦");
    assert_eq!(low["benchmark"], "安泽低硫主焦");
    assert_close(&low["points"], 1101.8);
    assert_close(&low["fair"], 2250.0 * 1101.8 / 1099.0);

    // 高硫肥煤对标肥煤基准, 不从 S1.3 主焦一路外推.
    let fat = value(&output, "高硫肥");
    assert_eq!(fat["benchmark"], "蒲县单一肥煤");
    assert_close(&fat["fair"], 2145.0 * 799.0 / 745.5);

    let gas = value(&output, "某气煤");
    assert_eq!(gas["benchmark"], "安泽低硫主焦");
    assert_close(&gas["fair"], 2250.0 * 1029.0 / 1099.0);

    // 标准品价格取最接近标准品的基准 (晋中, 离 1000 点只差 12.5 点) 反推.
    assert_close(&output["market_price"], 2050.0 * 1000.0 / 987.5);
}

#[test]
fn test_falls_back_to_market_price_without_benchmarks() {
    let output = estimate(&json!({
        "coals": coals(),
        "price_factor": {"adjusts": adjusts(), "market_price": 2000.0}
    }));
    assert_eq!(output["ok"], true, "{output}");
    let low = value(&output, "低硫焦");
    assert!(low.get("benchmark").is_none() || low["benchmark"].is_null());
    assert_close(&low["fair"], 2000.0 * 1101.8 / 1000.0);
    assert_close(&output["market_price"], 2000.0);
}

#[test]
fn test_rejects_invalid_benchmarks_and_input() {
    let mut bad = benchmarks();
    bad[0]["price"] = json!(-1.0);
    let output = estimate(&json!({
        "coals": coals(),
        "price_factor": {"adjusts": adjusts(), "benchmarks": bad}
    }));
    assert_eq!(output["ok"], false);
    assert!(
        output["reason"].as_str().unwrap().contains("晋中中硫主焦"),
        "{output}"
    );

    let mut duplicated = benchmarks();
    duplicated[1]["name"] = json!("晋中中硫主焦");
    let output = estimate(&json!({
        "coals": coals(),
        "price_factor": {"adjusts": adjusts(), "benchmarks": duplicated}
    }));
    assert_eq!(output["ok"], false);

    let output: Value = serde_json::from_str(&estimate_json("不是 JSON")).unwrap();
    assert_eq!(output["ok"], false);
}

#[test]
fn test_solver_uses_benchmark_prices_for_unpriced_coals() {
    let request = json!({
        "coals": [],
        "specs": [{"indicator": "S", "direction": "Upper", "max": 2.0, "enabled": true}],
        "total_quantity": 1000.0,
        "truncate_decimal": false,
        "price_factor": {
            "adjusts": adjusts(),
            "benchmarks": benchmarks(),
            "estimate_unpriced": true,
            "extra_coals": [coal("低硫焦", "焦煤",
                json!({"S": 0.6, "A": 9.7, "V": 22.0, "G": 96.0, "Y": 16.0,
                       "CSR": 70.0, "M": 10.0, "petro": 0.1}))]
        }
    });
    let result: Value = serde_json::from_str(&solve_json(&request.to_string())).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    assert_close(&result["cost"]["fob_per_ton"], 2250.0 * 1101.8 / 1099.0);
    assert_eq!(result["coal_values"][0]["benchmark"], "安泽低硫主焦");
}
