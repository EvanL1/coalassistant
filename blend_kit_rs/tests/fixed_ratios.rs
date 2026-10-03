//! 验算模式 (`BlendRequest.fixed_ratios`): 给定配比, 不求最优, 直接出成本与 8 项体检.
//!
//! 全部走公开 JSON 契约同一个入口 `solve`, 数据都是 fixture, 不读 master.

use blend_kit::*;
use std::collections::HashMap;

/// 顺序: (S, A, V, G, Y, petro, CSR, M, FOB, FRT)
fn four_coals() -> Vec<Coal> {
    vec![
        coal_from_tuple(
            "荣欣",
            (0.4, 9.7, 24.0, 97.0, 23.0, 0.1, 75.0, 10.0, 2400.0, 60.0),
        ),
        coal_from_tuple(
            "新星",
            (1.6, 10.5, 20.0, 75.0, 17.0, 0.1, 65.0, 10.0, 1550.0, 80.0),
        ),
        coal_from_tuple(
            "北沟",
            (1.8, 8.6, 24.0, 94.0, 22.0, 0.1, 55.0, 10.0, 2125.0, 70.0),
        ),
        coal_from_tuple(
            "浮精",
            (0.8, 12.0, 24.0, 80.0, 20.0, 0.1, 58.0, 12.0, 2465.0, 50.0),
        ),
    ]
}

fn shares(pairs: &[(&str, f64)]) -> Option<HashMap<String, f64>> {
    Some(
        pairs
            .iter()
            .map(|(name, share)| (name.to_string(), *share))
            .collect(),
    )
}

fn request(
    coals: Vec<Coal>,
    specs: Vec<Spec>,
    fixed: Option<HashMap<String, f64>>,
) -> BlendRequest {
    BlendRequest {
        coals,
        specs,
        total_quantity: None,
        truncate_decimal: false,
        fixed_ratios: fixed,
        rank_interaction: None,
    }
}

fn value_of(result: &BlendResult, indicator: &str) -> f64 {
    result
        .indicator_check
        .iter()
        .find(|check| check.indicator == indicator)
        .unwrap_or_else(|| panic!("体检里没有 {indicator}"))
        .value
}

fn check_of<'a>(result: &'a BlendResult, indicator: &str) -> &'a IndicatorCheck {
    result
        .indicator_check
        .iter()
        .find(|check| check.indicator == indicator)
        .unwrap_or_else(|| panic!("体检里没有 {indicator}"))
}

#[test]
fn test_fixed_ratios_give_weighted_indicators_and_cost() {
    let specs = vec![Spec::upper("S", 2.5), Spec::lower("G", 80.0)];
    let fixed = shares(&[("荣欣", 2.0), ("新星", 4.0), ("北沟", 3.0), ("浮精", 2.0)]);
    let result = solve(&request(four_coals(), specs, fixed));

    assert!(result.ok, "验算应当出结果: {:?}", result.reason);
    // 份数按总和 11 归一
    assert!((result.recipe["新星"] - 4.0 / 11.0).abs() < 1e-12);
    for (indicator, expected) in [
        ("S", 14.2 / 11.0),
        ("A", 111.2 / 11.0),
        ("V", 248.0 / 11.0),
        ("G", 936.0 / 11.0),
        ("Y", 220.0 / 11.0),
        ("CSR", 691.0 / 11.0),
    ] {
        let got = value_of(&result, indicator);
        assert!(
            (got - expected).abs() < 1e-9,
            "{indicator}: 得 {got}, 应 {expected}"
        );
    }
    let cif = (2.0 * 2460.0 + 4.0 * 1630.0 + 3.0 * 2195.0 + 2.0 * 2515.0) / 11.0;
    let cost = result.cost.expect("验算应当有成本");
    assert!((cost.cif_per_ton - cif).abs() < 1e-9);
}

#[test]
fn test_shares_are_normalized_so_2_2_equals_1_1() {
    let specs = vec![Spec::upper("S", 2.5)];
    let a = solve(&request(
        four_coals(),
        specs.clone(),
        shares(&[("荣欣", 2.0), ("北沟", 2.0)]),
    ));
    let b = solve(&request(
        four_coals(),
        specs,
        shares(&[("荣欣", 1.0), ("北沟", 1.0)]),
    ));
    assert!(a.ok && b.ok);
    for indicator in INDICATORS {
        assert!((value_of(&a, indicator) - value_of(&b, indicator)).abs() < 1e-12);
    }
    assert!((a.recipe["荣欣"] - 0.5).abs() < 1e-12);
    assert!(
        !a.recipe.contains_key("新星"),
        "没给份数的煤按 0 计, 不进配方"
    );
}

/// 求最优拿不到解时回"不可行"; 验算的配方是用户给的, 超标要逐项报出来, 而不是拒绝回答.
#[test]
fn test_violating_recipe_is_reported_not_refused() {
    let specs = vec![Spec::upper("S", 1.0), Spec::lower("G", 80.0)];
    let fixed = shares(&[("新星", 1.0), ("北沟", 1.0)]); // S = 1.7
    let result = solve(&request(four_coals(), specs, fixed));

    assert!(result.ok, "验算不该因超标而无结果: {:?}", result.reason);
    assert_eq!(check_of(&result, "S").status, EvaluationStatus::Fail);
    assert_eq!(result.quality_status, QualityStatus::NeedsReview);
}

/// 验算与求最优必须是同一套算法: 把求解器自己的最优配比喂回验算, 成本、扣款、
/// 每项指标与判定都要一致. 带计价条款, 让扣款这一路也被钉住.
#[test]
fn test_evaluating_the_optimum_reproduces_the_solver() {
    let coals = vec![
        coal_from_tuple(
            "便宜高灰",
            (0.8, 11.5, 24.0, 85.0, 18.0, 0.1, 62.0, 10.0, 1000.0, 50.0),
        ),
        coal_from_tuple(
            "贵低灰",
            (0.6, 8.0, 24.0, 88.0, 18.0, 0.1, 64.0, 10.0, 1400.0, 50.0),
        ),
    ];
    let tiers = vec![
        PenaltyTier {
            width: Some(1.0),
            rate: 5.0,
        },
        PenaltyTier {
            width: None,
            rate: 400.0,
        },
    ];
    let specs = vec![
        Spec {
            enforcement: Enforcement::Priced,
            penalty: Some(Penalty {
                tiers,
                reject: 12.0,
            }),
            ..Spec::upper("A", 10.0)
        },
        Spec::upper("S", 2.5),
    ];
    let optimum = solve(&request(coals.clone(), specs.clone(), None));
    assert!(optimum.ok, "{:?}", optimum.reason);
    let optimum_cost = optimum.cost.clone().unwrap();
    assert!(optimum_cost.penalty_per_ton > 1.0, "本用例要让扣款真的发生");

    let replay = solve(&request(coals, specs, Some(optimum.recipe.clone())));
    assert!(replay.ok, "{:?}", replay.reason);
    let replay_cost = replay.cost.clone().unwrap();
    assert!((replay_cost.net_per_ton - optimum_cost.net_per_ton).abs() < 1e-5);
    assert!((replay_cost.penalty_per_ton - optimum_cost.penalty_per_ton).abs() < 1e-5);
    for check in &optimum.indicator_check {
        let other = check_of(&replay, &check.indicator);
        assert!(
            (other.value - check.value).abs() < 1e-9,
            "{}",
            check.indicator
        );
        assert_eq!(other.status, check.status, "{}", check.indicator);
    }
    assert_eq!(replay.quality_status, optimum.quality_status);
}

#[test]
fn test_unknown_coal_is_rejected_by_name() {
    let result = solve(&request(
        four_coals(),
        vec![Spec::upper("S", 2.5)],
        shares(&[("不存在", 1.0)]),
    ));
    assert!(!result.ok);
    assert!(
        result.reason.as_deref().unwrap_or("").contains("不存在"),
        "{:?}",
        result.reason
    );
}

#[test]
fn test_negative_or_all_zero_shares_are_rejected() {
    for bad in [
        shares(&[("荣欣", -1.0), ("北沟", 2.0)]),
        shares(&[("荣欣", 0.0)]),
    ] {
        let result = solve(&request(four_coals(), vec![Spec::upper("S", 2.5)], bad));
        assert!(!result.ok);
    }
}

/// 缺合同要求指标的煤会被剔出煤池; 用户却给了它份数, 这时不能悄悄按 0 算.
#[test]
fn test_share_on_a_culled_coal_is_an_error() {
    let mut coals = four_coals();
    coals[1].props.remove("G");
    let result = solve(&request(
        coals,
        vec![Spec::lower("G", 80.0)],
        shares(&[("新星", 1.0), ("北沟", 1.0)]),
    ));
    assert!(!result.ok);
    assert!(
        result.reason.as_deref().unwrap_or("").contains("新星"),
        "{:?}",
        result.reason
    );
}
