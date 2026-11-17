// 集成测试整体是测试代码: 断言失败本来就要炸, 逐一写 let-else 只会稀释测试意图.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
//! 计价项的拒收线与合同界按同一条判定规则判.
//!
//! 拒收线是合同上的数字: 判定值 (按合同的截断 / 四舍五入规则取舍后的值) 越过它才算越线.
//! 求解 (LP 的悬崖行) 与体检曾经各用一条线 —— LP 按判定规则折算, 体检拿原值比 —— 于是
//! 默认的"保留 1 位小数、截断"下, 求解给出的配方过不了自己的体检. 这里钉住两边是同一条线:
//! 求解给出的配方体检不判越线; 判定值超出拒收线一格必判越线; 安全余量只收紧求解那一侧.

use blend_kit::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// 顺序: (S, A, V, G, Y, petro, CSR, M, FOB, FRT). 只有灰分与价格不同.
fn ash_coal(name: &str, ash: f64, fob: f64) -> Coal {
    coal_from_tuple(name, (0.5, ash, 25.0, 85.0, 15.0, 0.1, 65.0, 8.0, fob, 0.0))
}

/// 只有粘结不同的单煤.
fn cohesion_coal(cohesion: f64) -> Coal {
    coal_from_tuple(
        "单煤",
        (0.5, 9.0, 25.0, cohesion, 15.0, 0.1, 65.0, 8.0, 1000.0, 0.0),
    )
}

/// 一条计价约束: 超合同界每单位扣 20 元, 越过 `reject` 拒收.
fn priced(indicator: &str, direction: Direction, bound: f64, reject: f64) -> Spec {
    let mut spec = match direction {
        Direction::Lower => Spec::lower(indicator, bound),
        _ => Spec::upper(indicator, bound),
    };
    spec.enforcement = Enforcement::Priced;
    spec.penalty = Some(Penalty {
        tiers: vec![PenaltyTier {
            width: None,
            rate: 20.0,
        }],
        reject,
    });
    spec
}

/// 线上默认: 不单独写判定规则的项按"保留 1 位小数、截断"判定.
fn request(coals: Vec<Coal>, spec: Spec) -> BlendRequest {
    let request: Value = json!({
        "coals": coals,
        "specs": [spec],
        "total_quantity": 1000.0
    });
    serde_json::from_value(request).unwrap()
}

fn check<'a>(result: &'a BlendResult, indicator: &str) -> &'a IndicatorCheck {
    result
        .indicator_check
        .iter()
        .find(|check| check.indicator == indicator)
        .expect("体检应包含合同指标")
}

/// 按给定份数验算.
fn replay(request: &BlendRequest, shares: BTreeMap<String, f64>) -> BlendResult {
    let mut fixed = request.clone();
    fixed.fixed_ratios = Some(shares);
    solve(&fixed)
}

fn alone(name: &str) -> BTreeMap<String, f64> {
    [(name.to_string(), 1.0)].into_iter().collect()
}

/// 灰合同 ≤10、拒收线 11: 便宜的高灰煤配到拒收线为止.
fn ash_pool() -> Vec<Coal> {
    vec![
        ash_coal("好煤", 9.0, 1500.0),
        ash_coal("便宜高灰", 14.0, 900.0),
    ]
}

/// 复现: 求解把灰顶到 11.09999 (按截断记 11.0, 没越线), 体检却拿原值比 11, 判 Fail,
/// 整个结果成了"需要复核".
#[test]
fn test_solved_recipe_passes_its_own_check_under_truncation() {
    let request = request(ash_pool(), priced("A", Direction::Upper, 10.0, 11.0));
    let result = solve(&request);
    assert!(result.ok, "应可解: {:?}", result.reason);

    let ash = check(&result, "A");
    assert!(
        ash.value > 11.0 && ash.value < 11.1,
        "本例要让灰落在拒收线那一格里才有意义, 实得 {}",
        ash.value
    );
    assert_eq!(ash.judged_value, Some(11.0), "按截断记 11.0");
    assert_ne!(
        ash.status,
        EvaluationStatus::Fail,
        "判定值 11.0 没越过拒收线 11, 不该判越线 (原值 {})",
        ash.value
    );
    assert_ne!(
        result.quality_status,
        QualityStatus::NeedsReview,
        "求解给出的配方不该被自己的体检盖上需要复核"
    );
}

/// 验算路径与求解路径对同一个配方结论一致.
#[test]
fn test_replaying_the_solved_recipe_gives_the_same_verdict() {
    let request = request(ash_pool(), priced("A", Direction::Upper, 10.0, 11.0));
    let solved = solve(&request);
    assert!(solved.ok, "应可解: {:?}", solved.reason);

    let replayed = replay(&request, solved.recipe.clone());
    assert!(replayed.ok, "回放出错: {:?}", replayed.reason);
    assert_eq!(
        check(&replayed, "A").status,
        check(&solved, "A").status,
        "同一个配方, 验算与求解对灰的结论不同"
    );
    assert_ne!(check(&replayed, "A").status, EvaluationStatus::Fail);
    assert_eq!(replayed.quality_status, solved.quality_status);
    assert_ne!(replayed.quality_status, QualityStatus::NeedsReview);
}

/// 上界: 判定值超出拒收线一格必判越线, 还在拒收线那一格里的不判.
#[test]
fn test_one_cell_beyond_upper_reject_line_is_rejected() {
    let verdict = |ash: f64| {
        let request = request(
            vec![ash_coal("单煤", ash, 1000.0)],
            priced("A", Direction::Upper, 10.0, 11.0),
        );
        let evaluated = replay(&request, alone("单煤"));
        assert!(evaluated.ok, "验算出错: {:?}", evaluated.reason);
        (
            check(&evaluated, "A").status,
            evaluated.quality_status,
            solve(&request).ok,
        )
    };

    // 11.09 按截断记 11.0: 没越线. 验算认, 求解也解得出.
    let (status, quality, solvable) = verdict(11.09);
    assert_eq!(status, EvaluationStatus::TolerancePass);
    assert_ne!(quality, QualityStatus::NeedsReview);
    assert!(solvable, "灰 11.09 没越拒收线, 应可解");

    // 11.1 记 11.1: 越线一格. 验算判越线, 求解说配不出.
    let (status, quality, solvable) = verdict(11.1);
    assert_eq!(status, EvaluationStatus::Fail);
    assert_eq!(quality, QualityStatus::NeedsReview);
    assert!(!solvable, "灰 11.1 越过拒收线一格, 必须不可行");
}

/// 下界, 四舍五入: 粘结合同 ≥85、拒收线 80, 保留 1 位小数四舍五入.
/// 79.96 记 80.0 (没越线), 79.94 记 79.9 (越线一格).
#[test]
fn test_one_cell_beyond_lower_reject_line_is_rejected() {
    let verdict = |cohesion: f64| {
        let mut spec = priced("G", Direction::Lower, 85.0, 80.0);
        spec.acceptance = Some(AcceptanceRule {
            mode: AcceptanceMode::Round,
            decimals: Some(1),
            tolerance: 0.0,
        });
        let request = request(vec![cohesion_coal(cohesion)], spec);
        let evaluated = replay(&request, alone("单煤"));
        assert!(evaluated.ok, "验算出错: {:?}", evaluated.reason);
        (
            check(&evaluated, "G").status,
            evaluated.quality_status,
            solve(&request).ok,
        )
    };

    let (status, quality, solvable) = verdict(79.96);
    assert_eq!(status, EvaluationStatus::TolerancePass);
    assert_ne!(quality, QualityStatus::NeedsReview);
    assert!(solvable, "粘结 79.96 记 80.0, 没越拒收线, 应可解");

    let (status, quality, solvable) = verdict(79.94);
    assert_eq!(status, EvaluationStatus::Fail);
    assert_eq!(quality, QualityStatus::NeedsReview);
    assert!(!solvable, "粘结 79.94 记 79.9, 越过拒收线一格, 必须不可行");
}

/// 下界, 默认截断: 79.99 记 79.9, 越线; 80.0 正好压线, 不越.
#[test]
fn test_truncated_lower_reject_line_keeps_its_own_cell() {
    let status = |cohesion: f64| {
        let request = request(
            vec![cohesion_coal(cohesion)],
            priced("G", Direction::Lower, 85.0, 80.0),
        );
        let evaluated = replay(&request, alone("单煤"));
        (check(&evaluated, "G").status, solve(&request).ok)
    };
    assert_eq!(status(80.0), (EvaluationStatus::TolerancePass, true));
    assert_eq!(status(79.99), (EvaluationStatus::Fail, false));
}

/// 安全余量只收紧求解那一侧: 带余量解出的配方离拒收线更远, 体检照样不判越线;
/// 体检自己不带余量 —— 落在 [拒收线 − 余量, 拒收线] 里的给定配比, 验算仍认.
#[test]
fn test_margin_tightens_the_solver_but_not_the_check() {
    let mut spec = priced("A", Direction::Upper, 10.0, 11.0);
    spec.margin = Some(0.05);
    let request = request(ash_pool(), spec);
    let solved = solve(&request);
    assert!(solved.ok, "应可解: {:?}", solved.reason);

    let ash = check(&solved, "A");
    assert!(
        ash.value > 11.0 && ash.value < 11.051,
        "余量 0.05 应把灰挡在 11.05 以内, 实得 {}",
        ash.value
    );
    assert_ne!(ash.status, EvaluationStatus::Fail);
    assert_ne!(solved.quality_status, QualityStatus::NeedsReview);

    // 单煤灰 11.08: 判定值 11.0 没越线, 验算认; 求解那一侧被余量挡住 (执行界 11.05).
    let mut spec = priced("A", Direction::Upper, 10.0, 11.0);
    spec.margin = Some(0.05);
    let single = request_single_ash(11.08, spec);
    let evaluated = replay(&single, alone("单煤"));
    assert_eq!(
        check(&evaluated, "A").status,
        EvaluationStatus::TolerancePass
    );
    assert!(
        !solve(&single).ok,
        "余量把求解的拒收线收到 11.05, 灰 11.08 应不可行"
    );
}

fn request_single_ash(ash: f64, spec: Spec) -> BlendRequest {
    request(vec![ash_coal("单煤", ash, 1000.0)], spec)
}

/// 按原值判定的项不变: 原值比原始拒收线.
#[test]
fn test_raw_judging_still_compares_the_raw_value() {
    let status = |truncate_decimal: bool, acceptance: Option<AcceptanceRule>, ash: f64| {
        let mut spec = priced("A", Direction::Upper, 10.0, 11.0);
        spec.acceptance = acceptance;
        let mut request = request_single_ash(ash, spec);
        request.truncate_decimal = truncate_decimal;
        let evaluated = replay(&request, alone("单煤"));
        (check(&evaluated, "A").status, solve(&request).ok)
    };
    // 请求级关掉截断.
    assert_eq!(
        status(false, None, 11.0),
        (EvaluationStatus::TolerancePass, true)
    );
    assert_eq!(status(false, None, 11.01), (EvaluationStatus::Fail, false));
    // 这一项单独写明按原值判定.
    let raw = Some(AcceptanceRule::default());
    assert_eq!(
        status(true, raw.clone(), 11.0),
        (EvaluationStatus::TolerancePass, true)
    );
    assert_eq!(status(true, raw, 11.01), (EvaluationStatus::Fail, false));
}
