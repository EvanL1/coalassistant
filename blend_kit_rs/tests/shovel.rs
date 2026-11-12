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

/// 被剔出煤池的煤 (缺硬约束所需指标) 的配比下限不生效, 与 LP 侧一致: 凑整后的下限
/// 检查只针对实际在配方里的煤. 不然每组铲数都因"丙 0 < 0.2"被判不可用, 最后退回
/// 未凑整的配方.
#[test]
fn test_min_ratio_on_culled_coal_does_not_block_rounding() {
    let coals = vec![
        coal("甲", 0.6, 1500.0),
        coal("乙", 1.4, 1200.0),
        coal("丙", 1.0, 1100.0),
    ];
    let mut req = request(coals, 1.0);
    req.specs.push(Spec::lower("CSR", 58.0));
    req.total_quantity = Some(99.0);
    req.shovel_tons = Some(3.0);
    req.min_ratios.insert("丙".into(), 0.2);
    let csrless = req.coals.iter().position(|c| c.name == "丙").unwrap();
    req.coals[csrless].props.remove("CSR");
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert!(
        result
            .warnings
            .iter()
            .all(|w| !w.contains("凑不成整铲") && !w.contains("下限")),
        "{:?}",
        result.warnings
    );
    let total: u32 = result.orders.iter().filter_map(|order| order.shovels).sum();
    assert_eq!(total, 33, "{:?}", result.orders);
    assert_eq!(shovels_of(&result, "甲"), Some(17));
    assert_eq!(shovels_of(&result, "乙"), Some(16));
    assert!(result.orders.iter().all(|order| order.coal != "丙"));
}

#[test]
fn test_json_without_new_fields_still_parses() {
    let json = r#"{"coals":[],"specs":[],"total_quantity":null}"#;
    let parsed: BlendRequest = serde_json::from_str(json).unwrap();
    assert!(parsed.shovel_tons.is_none());
    assert!(parsed.min_ratios.is_empty());
}

/// 凑整铲不能把 min_ratios 凑丢: 低硫下限 25%, 一铲 20% (5 铲), 最大余数法给它 1 铲
/// 只剩 20%. 这种凑整不算成功, 要挪成 2 铲.
#[test]
fn test_rounding_respects_min_ratios() {
    let coals = vec![coal("高硫", 1.0, 1000.0), coal("低硫", 0.5, 1300.0)];
    let mut req = request(coals, 1.0);
    req.shovel_tons = Some(20.0);
    req.min_ratios.insert("低硫".into(), 0.25);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert!(
        ratio_of(&result, "低硫") >= 0.25 - 1e-9,
        "低硫 {}",
        ratio_of(&result, "低硫")
    );
    assert_eq!(shovels_of(&result, "低硫"), Some(2));
    assert_eq!(shovels_of(&result, "高硫"), Some(3));
}

/// 采购量不是一铲的整数倍: 33 铲实际 99 吨, 订单与成本仍按 100 吨算 —— 要如实提示差多少.
#[test]
fn test_quantity_not_multiple_of_shovel_warns() {
    let mut req = request(tiny_fixer(), 1.0);
    req.shovel_tons = Some(3.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    let total: u32 = result.orders.iter().filter_map(|order| order.shovels).sum();
    assert_eq!(total, 33);
    let tons: f64 = result.orders.iter().filter_map(|order| order.tons).sum();
    assert!((tons - 100.0).abs() < 1e-9, "订单吨数口径不变: {tons}");
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("33 铲") && w.contains("99 吨") && w.contains("少 1 吨")),
        "{:?}",
        result.warnings
    );
}

#[test]
fn test_quantity_multiple_of_shovel_stays_quiet() {
    let mut req = request(tiny_fixer(), 1.0);
    req.shovel_tons = Some(5.0);
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert!(
        result.warnings.iter().all(|w| !w.contains("整数倍")),
        "{:?}",
        result.warnings
    );
}

/// 配方顶在合同界上, 一点余地都没有: 按最大余数法凑整会把指标顶破, 挪一铲也救不回.
/// 这时给被顶破的那几项留一点余量重新配、再凑整, 得到的整铲配方按原合同验算仍达标,
/// 并如实说为此比不凑整贵了多少.
#[test]
fn test_rounding_gets_headroom_when_recipe_sits_on_the_bounds() {
    let request: BlendRequest =
        serde_json::from_str(include_str!("fixtures/shovel_needs_headroom.json")).unwrap();
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    assert!(
        result
            .orders
            .iter()
            .all(|order| order.shovels.is_some_and(|count| count >= 1)),
        "每种煤都该是整铲: {:?}",
        result.orders
    );
    let shovels: u32 = result.orders.iter().filter_map(|order| order.shovels).sum();
    assert_eq!(shovels, 1429, "10000 吨 / 一铲 7 吨");
    let extra = result
        .shovel_extra_cost
        .expect("留余量重配要报出多花了多少");
    assert!(extra >= 0.0, "{extra}");
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("为凑成整铲")),
        "{:?}",
        result.warnings
    );
    assert!(
        !result
            .warnings
            .iter()
            .any(|warning| warning.contains("凑不成整铲")),
        "{:?}",
        result.warnings
    );

    // 余量只为给凑整留地方: 凑好的配方按原合同回放, 不能有不达标的项.
    let mut replay = request.clone();
    replay.fixed_ratios = Some(result.recipe.clone());
    replay.shovel_tons = None;
    let review = solve(&replay);
    assert!(review.ok, "{:?}", review.reason);
    for check in &review.indicator_check {
        assert_ne!(
            check.status,
            EvaluationStatus::Fail,
            "回放配方 {} 判 Fail: {check:?}",
            check.indicator
        );
    }
}

/// 直接凑整就成的, 不走留余量重配: 不报多花的成本, 输出里也没有这个字段.
#[test]
fn test_plain_rounding_reports_no_extra_cost() {
    let coals = vec![coal("低硫", 0.6, 1900.0), coal("高硫", 2.0, 1500.0)];
    let mut request = request(coals, 1.5);
    request.total_quantity = Some(300.0);
    request.shovel_tons = Some(3.0);
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    assert!(result.orders.iter().all(|order| order.shovels.is_some()));
    assert!(result.shovel_extra_cost.is_none());
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("shovel_extra_cost"));
}

/// 留余量只对进了 LP 的约束 (硬约束、计价拒收线) 起作用: 凑整顶破的若是软约束,
/// 不给它留余量, 说明里也不提它.
#[test]
fn test_headroom_is_only_for_constraints_the_lp_enforces() {
    let request: BlendRequest =
        serde_json::from_str(include_str!("fixtures/shovel_soft_spec_broken.json")).unwrap();
    let soft: Vec<&str> = request
        .specs
        .iter()
        .filter(|spec| !matches!(spec.enforcement, Enforcement::Hard | Enforcement::Priced))
        .map(|spec| label_zh(&spec.indicator))
        .collect();
    assert!(!soft.is_empty(), "fixture 里要有软约束");
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    assert!(result.orders.iter().all(|order| order.shovels.is_some()));
    let note = result
        .warnings
        .iter()
        .find(|warning| warning.starts_with("为凑成整铲"))
        .expect("这份合同要留余量才凑得成整铲");
    for label in soft {
        assert!(
            !note.contains(label),
            "软约束 {label} 不该在留余量的说明里: {note}"
        );
    }
}
