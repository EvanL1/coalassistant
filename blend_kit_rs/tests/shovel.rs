//! 一铲最小量 (`BlendRequest.shovel_tons`) 与单煤配比下限 (`BlendRequest.min_ratios`).
//!
//! 现场用铲车配煤: 每种煤要么不用, 要么至少一铲, 且只能整铲配. 数据都是 fixture.

use blend_kit::*;
use std::collections::HashMap;

/// 顺序: (S, A, V, G, Y, petro, CSR, M, FOB, FRT)
fn coal(name: &str, sulfur: f64, fob: f64) -> Coal {
    coal_from_tuple(
        name,
        (sulfur, 10.0, 25.0, 80.0, 18.0, 0.1, 65.0, 9.0, fob, 0.0),
    )
}

fn request(coals: Vec<Coal>, sulfur_max: f64) -> BlendRequest {
    BlendRequest {
        coals,
        specs: vec![Spec::upper("S", sulfur_max)],
        total_quantity: Some(100.0),
        truncate_decimal: false,
        fixed_ratios: None,
        rank_interaction: None,
        price_factor: None,
        price_robustness: None,
        shovel_tons: None,
        min_ratios: HashMap::new(),
    }
}

fn ratio_of(result: &BlendResult, name: &str) -> f64 {
    result.recipe.get(name).copied().unwrap_or(0.0)
}

fn shovels_of(result: &BlendResult, name: &str) -> Option<u32> {
    result
        .orders
        .iter()
        .find(|order| order.coal == name)
        .and_then(|order| order.shovels)
}

fn sulfur(result: &BlendResult) -> f64 {
    result
        .indicator_check
        .iter()
        .find(|check| check.indicator == "S")
        .expect("体检里没有 S")
        .value
}

/// 便宜的高硫煤 + 一点点低硫煤压硫: 连续解里低硫煤只要 3.85%.
fn tiny_fixer() -> Vec<Coal> {
    vec![coal("高硫", 1.02, 1000.0), coal("低硫", 0.5, 1300.0)]
}

#[test]
fn test_without_shovel_tiny_share_is_kept() {
    let result = solve(&request(tiny_fixer(), 1.0));
    assert!(result.ok, "{:?}", result.reason);
    let share = ratio_of(&result, "低硫");
    assert!((share - 0.02 / 0.52).abs() < 1e-4, "低硫 {share}");
    assert!(result.orders.iter().all(|order| order.shovels.is_none()));
}

#[test]
fn test_shovel_raises_needed_coal_to_one_shovel() {
    // 一铲 5 吨, 采购 100 吨 = 20 铲. 低硫煤去不掉 (高硫单用超标), 只能提到 1 铲.
    let mut req = request(tiny_fixer(), 1.0);
    req.shovel_tons = Some(5.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert_eq!(shovels_of(&result, "低硫"), Some(1));
    assert_eq!(shovels_of(&result, "高硫"), Some(19));
    assert!((ratio_of(&result, "低硫") - 0.05).abs() < 1e-9);
    assert!(sulfur(&result) <= 1.0 + 1e-9);
}

#[test]
fn test_shovel_drops_coal_when_cheaper_than_raising() {
    // 一铲 10 吨 = 10%. 低硫提到 10% 要 1030 元/吨; 换中硫煤对半配只要 1015.
    let mut coals = tiny_fixer();
    coals.push(coal("中硫", 0.98, 1030.0));
    let mut req = request(coals, 1.0);
    let plain = solve(&req);
    assert!(ratio_of(&plain, "低硫") > 0.0, "连续解应当用到低硫");

    req.shovel_tons = Some(10.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert_eq!(ratio_of(&result, "低硫"), 0.0);
    assert_eq!(shovels_of(&result, "高硫"), Some(5));
    assert_eq!(shovels_of(&result, "中硫"), Some(5));
    let cost = result.cost.as_ref().unwrap().net_per_ton;
    assert!((cost - 1015.0).abs() < 1e-6, "成本 {cost}");
}

#[test]
fn test_whole_shovels_sum_to_purchase() {
    // 一铲 3 吨, 采购 100 吨 → 33 铲; 每种煤整铲, 配比 = 铲数 / 33.
    let mut coals = tiny_fixer();
    coals.push(coal("中硫", 0.98, 1030.0));
    let mut req = request(coals, 1.0);
    req.shovel_tons = Some(3.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    let total: u32 = result.orders.iter().filter_map(|order| order.shovels).sum();
    assert_eq!(total, 33);
    for order in &result.orders {
        let shovels = order.shovels.expect("每行都要有铲数");
        assert!(shovels >= 1);
        assert!((order.ratio - f64::from(shovels) / 33.0).abs() < 1e-9);
    }
    assert!(sulfur(&result) <= 1.0 + 1e-9);
}

#[test]
fn test_rounding_that_breaks_spec_is_repaired() {
    // 一铲 10 吨, 采购 30 吨 → 3 铲. 连续解低硫 40% = 1.2 铲, 四舍五入成 1 铲硫超标,
    // 要挪成 2 铲.
    let coals = vec![coal("高硫", 1.2, 1000.0), coal("低硫", 0.7, 1300.0)];
    let mut req = request(coals, 1.0);
    req.total_quantity = Some(30.0);
    req.shovel_tons = Some(10.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert_eq!(shovels_of(&result, "低硫"), Some(2));
    assert_eq!(shovels_of(&result, "高硫"), Some(1));
    assert!(sulfur(&result) <= 1.0 + 1e-9);
}

#[test]
fn test_shovel_bigger_than_purchase_is_ignored_with_warning() {
    // 一铲比采购量还大: 一铲都装不下, 不按铲配, 提示一句照常求解.
    let mut req = request(tiny_fixer(), 1.0);
    req.shovel_tons = Some(150.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert!(ratio_of(&result, "低硫") < 0.05);
    assert!(result.warnings.iter().any(|w| w.contains("比采购总量")));
}

#[test]
fn test_rounding_rechecks_exact_petrography() {
    // 7 铲. LP 解里 P2 占 21.7%, σ 0.192; 凑成 2 铲 (28.6%) 后真实 σ ≈ 0.214 越过 Hard 0.2
    // (线性代理 Σxσ 看不出来). 必须按真实配比复核, 挪成 1 铲.
    let req: BlendRequest = serde_json::from_value(serde_json::json!({
        "coals": [
            {"name": "P1", "props": {"S": 0.5, "A": 10, "V": 25, "G": 80, "Y": 18, "CSR": 65, "M": 9},
             "fob": 1300, "frt": 0, "petrography": {"vitrinite_pct": 60, "mean": 1.0, "std_dev": 0.05}},
            {"name": "P2", "props": {"S": 0.5, "A": 10, "V": 25, "G": 80, "Y": 18, "CSR": 65, "M": 9},
             "fob": 1000, "frt": 0, "petrography": {"vitrinite_pct": 60, "mean": 1.3, "std_dev": 0.3}}
        ],
        "specs": [{"indicator": "petro", "direction": "Upper", "min": null, "max": 0.2}],
        "total_quantity": 100,
        "truncate_decimal": false,
        "shovel_tons": 14.2857
    }))
    .unwrap();
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert_eq!(shovels_of(&result, "P2"), Some(1));
    assert_eq!(shovels_of(&result, "P1"), Some(6));
    let petro = result.petrography_check.as_ref().expect("要有岩相精确复核");
    assert!(petro.sigma <= 0.2, "σ {}", petro.sigma);
    assert_eq!(petro.sigma_ok, Some(true));
    assert!(result.warnings.iter().all(|w| !w.contains("验算模式")));
}

#[test]
fn test_zero_min_ratio_does_not_block_dropping() {
    // min_ratios 里给 0 = 不限; 分支定界去掉这种煤时不能因为键还在而校验失败.
    let mut coals = tiny_fixer();
    coals.push(coal("中硫", 0.98, 1030.0));
    let mut req = request(coals, 1.0);
    req.shovel_tons = Some(10.0);
    req.min_ratios.insert("低硫".into(), 0.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert_eq!(ratio_of(&result, "低硫"), 0.0);
    let cost = result.cost.as_ref().unwrap().net_per_ton;
    assert!((cost - 1015.0).abs() < 1e-6, "成本 {cost}");
}

#[test]
fn test_robustness_resolves_in_whole_shovels() {
    // 稳健性重解也要按整铲配: 零改价时与基准配方相同, 门槛不会全塌到第一档,
    // 更稳的配方 (有的话) 也都是整铲.
    let mut coals = tiny_fixer();
    coals.push(coal("中硫", 0.98, 1030.0));
    let mut req = request(coals, 1.0);
    req.shovel_tons = Some(10.0);
    req.price_robustness = Some(PriceRobustness {
        tolerance: 10.0,
        swing_range: 300.0,
    });
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    let report = result.price_robustness.as_ref().expect("要有稳健性报告");
    assert!(
        !report
            .swings
            .iter()
            .all(|swing| swing.rise == Some(5.0) && swing.fall == Some(5.0)),
        "{:?}",
        report.swings
    );
    if let Some(robust) = &report.robust {
        for share in robust.recipe.values() {
            assert!(
                (share * 10.0 - (share * 10.0).round()).abs() < 1e-6,
                "{share}"
            );
        }
    }
}

#[test]
fn test_shovel_without_quantity_is_ignored_with_warning() {
    let mut req = request(tiny_fixer(), 1.0);
    req.total_quantity = None;
    req.shovel_tons = Some(5.0);
    let result = solve(&req);
    assert!(result.ok);
    assert!(ratio_of(&result, "低硫") < 0.05);
    assert!(result.warnings.iter().any(|w| w.contains("一铲")));
}

#[test]
fn test_min_ratios_force_coal_in() {
    let mut req = request(tiny_fixer(), 1.0);
    req.min_ratios.insert("低硫".into(), 0.2);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert!((ratio_of(&result, "低硫") - 0.2).abs() < 1e-6);
}

#[test]
fn test_invalid_shovel_and_min_ratios_are_rejected() {
    let cases: Vec<(BlendRequest, &str)> = vec![
        {
            let mut req = request(tiny_fixer(), 1.0);
            req.shovel_tons = Some(0.0);
            (req, "一铲")
        },
        {
            let mut req = request(tiny_fixer(), 1.0);
            req.min_ratios.insert("不存在".into(), 0.1);
            (req, "不存在")
        },
        {
            let mut req = request(tiny_fixer(), 1.0);
            req.min_ratios.insert("低硫".into(), 1.5);
            (req, "低硫")
        },
        {
            let mut req = request(tiny_fixer(), 1.0);
            req.min_ratios.insert("低硫".into(), 0.6);
            req.min_ratios.insert("高硫".into(), 0.6);
            (req, "合计")
        },
    ];
    for (req, needle) in cases {
        let result = solve(&req);
        assert!(!result.ok);
        let reason = result.reason.unwrap_or_default();
        assert!(reason.contains(needle), "{reason} 应含 {needle}");
    }
}

#[test]
fn test_json_without_new_fields_still_parses() {
    let json = r#"{"coals":[],"specs":[],"total_quantity":null}"#;
    let parsed: BlendRequest = serde_json::from_str(json).unwrap();
    assert!(parsed.shovel_tons.is_none());
    assert!(parsed.min_ratios.is_empty());
}
