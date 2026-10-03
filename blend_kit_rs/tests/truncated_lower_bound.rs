//! 回归: 有解的合同被报成"约束冲突, LP 不可行".
//!
//! 两种煤, S≤2.5 且 G≥80, 单用荣欣就满足. App 固定传 truncate_decimal=true
//! (一位小数截断判定), LP 解出 G=79.99999998857, 截断成 79.9 → 复核判 Fail →
//! 整单丢弃. 根因: 下界的执行界恰好压在格线上, 没像上界那样往里留缓冲.

use blend_kit::*;

#[test]
fn test_binding_lower_bound_under_truncation_is_feasible() {
    let request = BlendRequest {
        coals: vec![
            coal_from_tuple(
                "荣欣",
                (0.4, 9.7, 24.0, 97.0, 23.0, 0.1, 75.0, 10.0, 2400.0, 60.0),
            ),
            coal_from_tuple(
                "新星",
                (1.6, 10.5, 20.0, 75.0, 17.0, 0.1, 65.0, 10.0, 1550.0, 80.0),
            ),
        ],
        specs: vec![Spec::upper("S", 2.5), Spec::lower("G", 80.0)],
        total_quantity: None,
        truncate_decimal: true,
        fixed_ratios: None,
        rank_interaction: None,
    };
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    let g = result
        .indicator_check
        .iter()
        .find(|check| check.indicator == "G")
        .unwrap();
    assert!(
        g.judged_value.unwrap() >= 80.0,
        "G 判定值 {:?}",
        g.judged_value
    );
    assert_ne!(g.status, EvaluationStatus::Fail);
}

/// 化验值常是整数: G 正好 80 的煤对 G≥80 必须可行 (80.0 截断仍是 80.0).
/// 防的是"把下界执行界往里缩一个 eps"那种修法 —— 它会恰好卡掉最常见的整数值.
#[test]
fn test_assay_exactly_on_lower_bound_stays_feasible() {
    let request = BlendRequest {
        coals: vec![coal_from_tuple(
            "正好80",
            (0.8, 9.0, 24.0, 80.0, 15.0, 0.1, 62.0, 10.0, 1500.0, 50.0),
        )],
        specs: vec![Spec::lower("G", 80.0), Spec::lower("Y", 15.0)],
        total_quantity: None,
        truncate_decimal: true,
        fixed_ratios: None,
        rank_interaction: None,
    };
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
}
