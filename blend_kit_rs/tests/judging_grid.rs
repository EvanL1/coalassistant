// 集成测试整体是测试代码: 断言失败本来就要炸, 逐一写 let-else 只会稀释测试意图.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
//! 判定贴格: 刚好超出合同一格的值必须判不达标, 求解时要指认出是哪一项够不到.
//!
//! 合同按 ≥2 位小数判定时, 求解器残差的放行量 (1e-7 × 量级) 曾经盖过执行界留的
//! 缓冲, 把"界外一格"的值吸回合同那一格: 验算判通过, 求解报"约束冲突"却一项都
//! 指认不出来.

use blend_kit::*;

/// (指标, 单煤化验值, 方向, 合同界, 判定方式, 小数位).
type Case = (&'static str, f64, Direction, f64, AcceptanceMode, u8);

/// 每一条都恰好在合同界外一格 (四舍五入的上界取在进位线上).
const ONE_GRID_BEYOND: [Case; 9] = [
    (
        "A",
        10.51,
        Direction::Upper,
        10.50,
        AcceptanceMode::Truncate,
        2,
    ),
    (
        "V",
        28.01,
        Direction::Upper,
        28.00,
        AcceptanceMode::Truncate,
        2,
    ),
    (
        "A",
        10.495,
        Direction::Upper,
        10.49,
        AcceptanceMode::Round,
        2,
    ),
    (
        "S",
        0.850,
        Direction::Upper,
        0.849,
        AcceptanceMode::Truncate,
        3,
    ),
    (
        "S",
        0.81,
        Direction::Upper,
        0.80,
        AcceptanceMode::Truncate,
        2,
    ),
    (
        "A",
        10.5,
        Direction::Upper,
        10.4,
        AcceptanceMode::Truncate,
        1,
    ),
    (
        "G",
        99.9,
        Direction::Lower,
        100.0,
        AcceptanceMode::Truncate,
        1,
    ),
    (
        "G",
        79.99,
        Direction::Lower,
        80.0,
        AcceptanceMode::Truncate,
        2,
    ),
    ("G", 79.99, Direction::Lower, 80.0, AcceptanceMode::Round, 2),
];

/// 正好压在合同界上: 必须达标.
const ON_THE_BOUND: [Case; 6] = [
    (
        "A",
        10.50,
        Direction::Upper,
        10.50,
        AcceptanceMode::Truncate,
        2,
    ),
    (
        "V",
        28.00,
        Direction::Upper,
        28.00,
        AcceptanceMode::Truncate,
        2,
    ),
    (
        "A",
        10.49,
        Direction::Upper,
        10.49,
        AcceptanceMode::Round,
        2,
    ),
    (
        "S",
        0.849,
        Direction::Upper,
        0.849,
        AcceptanceMode::Truncate,
        3,
    ),
    (
        "G",
        100.0,
        Direction::Lower,
        100.0,
        AcceptanceMode::Truncate,
        1,
    ),
    ("G", 80.0, Direction::Lower, 80.0, AcceptanceMode::Round, 2),
];

fn request_for(case: &Case, fixed: bool) -> BlendRequest {
    let (indicator, value, direction, bound, mode, decimals) = *case;
    let mut coal = coal_from_tuple(
        "单煤",
        (0.5, 9.0, 24.0, 85.0, 16.0, 0.1, 65.0, 10.0, 1500.0, 50.0),
    );
    coal.props.insert(indicator.into(), value);
    let mut spec = match direction {
        Direction::Lower => Spec::lower(indicator, bound),
        _ => Spec::upper(indicator, bound),
    };
    spec.acceptance = Some(AcceptanceRule {
        mode,
        decimals: Some(decimals),
        tolerance: 0.0,
    });
    BlendRequest {
        coals: vec![coal],
        specs: vec![spec],
        total_quantity: None,
        truncate_decimal: false,
        fixed_ratios: fixed.then(|| [("单煤".to_string(), 1.0)].into_iter().collect()),
        rank_interaction: None,
        price_factor: None,
        price_robustness: None,
        shovel_tons: None,
        min_ratios: Default::default(),
    }
}

fn check_of<'a>(result: &'a BlendResult, indicator: &str) -> &'a IndicatorCheck {
    result
        .indicator_check
        .iter()
        .find(|check| check.indicator == indicator)
        .expect("体检应包含合同指标")
}

#[test]
fn test_one_grid_beyond_contract_fails_when_evaluated() {
    for case in &ONE_GRID_BEYOND {
        let (indicator, value, direction, bound, mode, decimals) = *case;
        let label =
            format!("{indicator}={value} 对合同 {direction:?} {bound} ({mode:?} {decimals} 位)");
        let result = solve(&request_for(case, true));
        assert!(
            result.ok,
            "{label}: 验算应给出体检, 实得 {:?}",
            result.reason
        );
        let check = check_of(&result, indicator);
        assert_eq!(
            check.status,
            EvaluationStatus::Fail,
            "{label}: 界外一格必须判不达标"
        );
        let judged = check.judged_value.expect("应有判定值");
        let beyond = match direction {
            Direction::Lower => judged < bound,
            _ => judged > bound,
        };
        assert!(
            beyond,
            "{label}: 判不达标时展示的判定值不能还在合同界内, 实得 {judged}"
        );
        assert_eq!(result.quality_status, QualityStatus::NeedsReview, "{label}");
    }
}

#[test]
fn test_one_grid_beyond_contract_is_diagnosed_when_solved() {
    for case in &ONE_GRID_BEYOND {
        let (indicator, value, direction, bound, mode, decimals) = *case;
        let label =
            format!("{indicator}={value} 对合同 {direction:?} {bound} ({mode:?} {decimals} 位)");
        let result = solve(&request_for(case, false));
        assert!(!result.ok, "{label}: 唯一的煤在界外一格, 不该解出配方");
        assert_eq!(
            result.reason.as_deref(),
            Some("约束冲突, LP 不可行"),
            "{label}"
        );
        assert!(!result.undetermined, "{label}: 冲突是确定的");
        assert_eq!(
            result.infeasible_bounds.len(),
            1,
            "{label}: 应指认这一项, 实得 {:?}",
            result.infeasible_bounds
        );
        let culprit = &result.infeasible_bounds[0];
        assert_eq!(culprit.indicator, indicator, "{label}");
        assert_eq!(culprit.direction, direction, "{label}");
        assert!(
            (culprit.achievable - value).abs() < 1e-6,
            "{label}: 最好只能做到单煤的化验值, 实得 {}",
            culprit.achievable
        );
    }
}

#[test]
fn test_value_on_contract_bound_passes_both_ways() {
    for case in &ON_THE_BOUND {
        let (indicator, value, direction, bound, mode, decimals) = *case;
        let label =
            format!("{indicator}={value} 对合同 {direction:?} {bound} ({mode:?} {decimals} 位)");
        let evaluated = solve(&request_for(case, true));
        assert_ne!(
            check_of(&evaluated, indicator).status,
            EvaluationStatus::Fail,
            "{label}: 验算"
        );
        let solved = solve(&request_for(case, false));
        assert!(solved.ok, "{label}: 求解 {:?}", solved.reason);
        assert_ne!(
            check_of(&solved, indicator).status,
            EvaluationStatus::Fail,
            "{label}: 求解"
        );
    }
}

/// 界内的值, 展示的判定值就是它按规则落到的那一格 —— 不许因为"先推一个放行量再量化"
/// 而少一格. 放行量是给贴着执行界的求解器残差用的; 曾经对所有界内的值都推, 正好压在
/// 格线上的值 (化验值与简单配比的混合值常常如此) 于是被推到前一格: 灰分 10.30 对合同
/// ≤10.5 显示"判定值 10.2", 按原值判定的项则整体偏出一个放行量 (4 显示成 4.0000005).
/// 达标结论不受影响, 错的是展示出来、并被 Agent 念给客户听的那个数.
#[test]
fn test_judged_value_of_an_in_bound_value_is_its_own_grid_cell() {
    // (指标, 单煤化验值, 方向, 合同界, 判定方式, 小数位, 应展示的判定值).
    let cases: [(Case, f64); 7] = [
        (
            (
                "A",
                10.3,
                Direction::Upper,
                10.5,
                AcceptanceMode::Truncate,
                1,
            ),
            10.3,
        ),
        (
            (
                "M",
                10.0,
                Direction::Upper,
                12.0,
                AcceptanceMode::Truncate,
                1,
            ),
            10.0,
        ),
        (
            (
                "A",
                10.50,
                Direction::Upper,
                10.50,
                AcceptanceMode::Truncate,
                2,
            ),
            10.50,
        ),
        // 四舍五入: 正好在进位线上的值进到下一格, 不是被推回前一格.
        (
            ("A", 10.25, Direction::Upper, 10.5, AcceptanceMode::Round, 1),
            10.3,
        ),
        // 按原值判定: 判定值就是原值, 界内、界上都一样.
        (
            ("S", 1.2, Direction::Upper, 2.0, AcceptanceMode::Raw, 0),
            1.2,
        ),
        (
            ("S", 4.0, Direction::Lower, 4.0, AcceptanceMode::Raw, 0),
            4.0,
        ),
        (
            ("G", 85.0, Direction::Lower, 80.0, AcceptanceMode::Raw, 0),
            85.0,
        ),
    ];
    for (case, expected) in &cases {
        let (indicator, value, direction, bound, mode, decimals) = *case;
        let label =
            format!("{indicator}={value} 对合同 {direction:?} {bound} ({mode:?} {decimals} 位)");
        let result = solve(&request_for(case, true));
        let check = check_of(&result, indicator);
        assert_ne!(check.status, EvaluationStatus::Fail, "{label}");
        assert_eq!(
            check.judged_value,
            Some(*expected),
            "{label}: 判定值应是原值按规则落到的那一格"
        );
    }
}
