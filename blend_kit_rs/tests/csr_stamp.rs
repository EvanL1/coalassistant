//! 捣固炼焦 CSR 估算 (ρ=1.0, 不含 MCI 项): 只展示, 不参与求解.

use blend_kit::*;
use std::collections::HashMap;

/// 按公式系数手算: M40 = 87.9924, CSR = 68.573.
/// 输入取朋友配方 2:4:3:2 的配合煤实测值 (Vdaf 22.14 / G 87 / Y 20), 实测焦炭 CSR 70.
#[test]
fn test_formula_matches_published_coefficients() {
    let csr = predict::csr_stamp_charging(22.14, 87.0, 20.0, None);
    assert!((csr - 68.573).abs() < 0.01, "得 {csr}");
}

fn coals() -> Vec<Coal> {
    vec![
        coal_from_tuple(
            "荣欣",
            (0.4, 9.7, 24.0, 97.0, 23.0, 0.1, 75.0, 10.0, 2500.0, 0.0),
        ),
        coal_from_tuple(
            "新星",
            (1.6, 10.5, 20.0, 75.0, 17.0, 0.1, 65.0, 10.0, 2100.0, 0.0),
        ),
        coal_from_tuple(
            "北沟",
            (1.8, 8.6, 24.0, 94.0, 22.0, 0.1, 55.0, 10.0, 2200.0, 0.0),
        ),
        coal_from_tuple(
            "浮精",
            (0.8, 12.0, 24.0, 80.0, 20.0, 0.1, 58.0, 12.0, 2000.0, 0.0),
        ),
    ]
}

fn request(coals: Vec<Coal>, fixed: Option<HashMap<String, f64>>) -> BlendRequest {
    BlendRequest {
        coals,
        specs: vec![Spec::upper("S", 2.5)],
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
        .unwrap()
        .value
}

/// 结果里的估算值必须由同一份结果里展示的挥发/G/Y 算出, 两处数字对得上.
#[test]
fn test_result_carries_estimate_from_its_own_blend_indicators() {
    let fixed: HashMap<String, f64> = [("荣欣", 2.0), ("新星", 4.0), ("北沟", 3.0), ("浮精", 2.0)]
        .into_iter()
        .map(|(name, share)| (name.to_string(), share))
        .collect();
    let result = solve(&request(coals(), Some(fixed)));
    assert!(result.ok);
    let expected = predict::csr_stamp_charging(
        value_of(&result, "V"),
        value_of(&result, "G"),
        value_of(&result, "Y"),
        None,
    );
    let got = result.csr_stamp_estimate.expect("挥发/G/Y 齐全时应有估算");
    assert!((got - expected).abs() < 1e-9);
}

/// 只展示: 加了估算后求解结果 (配方与成本) 不变 —— 它不进 LP.
#[test]
fn test_estimate_does_not_change_the_optimum() {
    let result = solve(&request(coals(), None));
    assert!(result.ok);
    assert!(result.csr_stamp_estimate.is_some());
    // 只有硫 ≤2.5 一条约束, 最优就是全用最便宜的浮精 (2000). 估算值偏爱高 G/Y 的煤,
    // 它若混进了目标函数, 这里就不会是浮精.
    assert!(
        (result.recipe["浮精"] - 1.0).abs() < 1e-6,
        "{:?}",
        result.recipe
    );
}

#[test]
fn test_no_estimate_when_an_input_is_missing() {
    let mut pool = coals();
    pool[0].props.remove("Y");
    let result = solve(&request(pool, None));
    assert!(result.ok);
    assert_eq!(result.csr_stamp_estimate, None, "缺 Y 就不编一个数");
}

#[test]
fn test_no_estimate_when_infeasible() {
    let mut req = request(coals(), None);
    req.specs = vec![Spec::upper("S", 0.1)];
    let result = solve(&req);
    assert!(!result.ok);
    assert_eq!(result.csr_stamp_estimate, None);
}

/// MCI 以百分数计 (常见取值 1~8), 所以要乘 100。手算: 分子 5+1.85·1+2.2·0.5+1.6·3+0.83·1+0.9·0.1 = 13.67,
/// Vd = 25×(100−10)/100 = 22.5, 分母 (100−22.5)×(50+0.41·30+2.5·1) = 5022,
/// MCI = 100 × 10 × 13.67 / 5022 = 2.7220.
#[test]
fn test_mineral_catalysis_index_in_percent() {
    let ash = predict::AshComposition {
        fe2o3: 5.0,
        k2o: 1.0,
        na2o: 0.5,
        cao: 3.0,
        mgo: 1.0,
        mno: 0.1,
        sio2: 50.0,
        al2o3: 30.0,
        tio2: 1.0,
    };
    let mci = predict::mineral_catalysis_index(10.0, 25.0, &ash);
    assert!((mci - 2.7220).abs() < 1e-3, "得 {mci}");
}

/// MCI 修正: 相对柳林基准 δ=1.0, 每高 1 扣 5.7。没有灰成分 (None) 时不修正.
#[test]
fn test_mci_term_only_applies_when_given() {
    let base = predict::csr_stamp_charging(22.14, 87.0, 20.0, None);
    let at_baseline = predict::csr_stamp_charging(22.14, 87.0, 20.0, Some(1.0));
    let alkaline = predict::csr_stamp_charging(22.14, 87.0, 20.0, Some(3.0));
    assert!((base - at_baseline).abs() < 1e-12);
    assert!(
        (base - alkaline - 11.4).abs() < 1e-9,
        "MCI 3 应比基准低 11.4"
    );
}
