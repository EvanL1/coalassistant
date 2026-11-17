// 集成测试整体是测试代码: 断言失败本来就要炸, 逐一写 let-else 只会稀释测试意图.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
//! 输出逐字节确定: 同一请求连调多次 `solve_json`, 输出字符串必须逐字节相等.
//!
//! 输出里的映射 (recipe / 稳健配方 / 配煤价值) 曾经用 HashMap: std 给每个 HashMap
//! 实例一颗新种子, 迭代顺序随创建次序变, 同一输入在不同调用里序列化出不同字节串.

use blend_kit::solve_json;
use serde_json::{json, Value};

fn coal(name: &str, s: f64, a: f64, fob: f64) -> Value {
    json!({
        "name": name, "fob": fob, "frt": 0.0,
        "props": {"S": s, "A": a, "V": 25.0, "G": 85.0, "Y": 15.0, "petro": 0.1, "CSR": 65.0, "M": 10.0}
    })
}

fn coals() -> Vec<Value> {
    vec![
        coal("甲", 0.5, 9.5, 1000.0),
        coal("乙", 1.5, 11.0, 903.0),
        coal("丙", 0.8, 8.5, 1052.0),
        coal("丁", 1.2, 12.0, 960.0),
    ]
}

fn cases() -> [(&'static str, Value); 4] {
    let specs = json!([
        {"indicator": "S", "direction": "Upper", "max": 1.0, "enabled": true},
        {"indicator": "A", "direction": "Upper", "max": 10.5, "enabled": true,
         "enforcement": "Priced",
         "penalty": {"tiers": [{"rate": 80.0}], "reject": 12.0}}
    ]);
    let plain = json!({
        "coals": coals(),
        "specs": [{"indicator": "S", "direction": "Upper", "max": 1.0, "enabled": true}],
        "total_quantity": 1000.0,
        "truncate_decimal": false
    });
    let priced = json!({
        "coals": coals(),
        "specs": specs,
        "total_quantity": 1000.0,
        "truncate_decimal": false
    });
    let mut robust = plain.clone();
    robust["price_robustness"] = json!({});
    let evaluated = json!({
        "coals": coals(),
        "specs": [{"indicator": "S", "direction": "Upper", "max": 1.0, "enabled": true}],
        "truncate_decimal": false,
        "fixed_ratios": {"甲": 1.0, "乙": 2.0, "丙": 3.0, "丁": 4.0}
    });
    [
        ("求解", plain),
        ("计价", priced),
        ("价格稳健性", robust),
        ("验算", evaluated),
    ]
}

#[test]
fn test_same_request_same_bytes() {
    for (label, request) in cases() {
        let input = request.to_string();
        let first = solve_json(&input);
        assert!(first.contains("\"ok\":true"), "{label}: {first}");
        for round in 0..20 {
            let again = solve_json(&input);
            assert_eq!(again, first, "{label} 第 {round} 次重算字节不同");
        }
    }
}

/// 一种煤同时缺好几项合同要用的指标: 剔煤警告里要把缺的指标列出来, 这串文字的顺序也得固定
/// (曾经从 HashSet 里拼出来, 同一请求连调几十次能得到十几种写法), 并且按指标的固定顺序排.
#[test]
fn test_missing_indicator_warning_is_stable() {
    let mut pool = coals();
    pool.push(json!({
        "name": "缺项", "fob": 900.0, "frt": 0.0,
        "props": {"V": 25.0, "petro": 0.1, "CSR": 65.0}
    }));
    let request = json!({
        "coals": pool,
        "specs": [
            {"indicator": "M", "direction": "Upper", "max": 12.0, "enabled": true},
            {"indicator": "Y", "direction": "Lower", "min": 10.0, "enabled": true},
            {"indicator": "G", "direction": "Lower", "min": 70.0, "enabled": true},
            {"indicator": "A", "direction": "Upper", "max": 11.0, "enabled": true},
            {"indicator": "S", "direction": "Upper", "max": 1.0, "enabled": true}
        ],
        "total_quantity": 1000.0,
        "truncate_decimal": false
    })
    .to_string();
    let first = solve_json(&request);
    assert!(first.contains("\"ok\":true"), "{first}");
    let parsed: Value = serde_json::from_str(&first).unwrap();
    let warning = parsed["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|text| text.starts_with("剔除 缺项"))
        .unwrap_or_else(|| panic!("没有剔煤警告: {first}"))
        .to_string();
    // 顺序是指标的固定顺序 (硫、灰、粘结、胶质、水分), 不是合同里写的顺序, 也不是哈希顺序.
    let order: Vec<usize> = ["硫", "灰", "粘", "胶", "水"]
        .iter()
        .map(|label| {
            warning
                .find(label)
                .unwrap_or_else(|| panic!("警告里没有 {label}: {warning}"))
        })
        .collect();
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "缺的指标没按固定顺序列出: {warning}"
    );
    for round in 0..50 {
        assert_eq!(solve_json(&request), first, "第 {round} 次重算字节不同");
    }
}
