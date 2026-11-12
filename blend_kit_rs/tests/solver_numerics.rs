//! 求解器数值边缘: 可行的合同不能报成"约束冲突", 求解器没收敛也不能说成"约束冲突".
//!
//! fixture 由随机生成的可行合同化简而来, 煤名不指向真实矿; 价格已是定价, 不走估价.

use blend_kit::*;
use std::collections::HashMap;

const CONFLICT: &str = "约束冲突, LP 不可行";

fn load(json: &str) -> BlendRequest {
    serde_json::from_str(json).expect("fixture 应能解析")
}

/// 解出来的配方要经得起复核: 把返回的 `recipe` 原样回放给验算路径 (`fixed_ratios`),
/// **不放宽**合同界 —— 客户照着配方配出来的煤必须按合同判定达标. 配比 ≤1e-5 的零头
/// 不进 recipe 也不进订单, 达标不能靠它们.
fn assert_recipe_passes_review(request: &BlendRequest, result: &BlendResult) {
    assert!(result.ok, "可行合同应有解: {:?}", result.reason);
    let total: f64 = result.recipe.values().sum();
    assert!((total - 1.0).abs() < 1e-3, "配比合计 {total}");
    assert!(result.recipe.values().all(|ratio| *ratio >= 0.0));
    let mut replay = request.clone();
    replay.fixed_ratios = Some(result.recipe.clone());
    replay.shovel_tons = None;
    replay.min_ratios = HashMap::new();
    let review = solve(&replay);
    assert!(review.ok, "回放验算失败: {:?}", review.reason);
    for check in &review.indicator_check {
        assert_ne!(
            check.status,
            EvaluationStatus::Fail,
            "回放配方 {} 判 Fail: {check:?}",
            check.indicator
        );
    }
}

/// 机制 A: 求解器报 Solved, 夹 0 + 归一化把顶格行的残差放大过了复核放行量.
/// 合同本身严格可行, 不能报成"约束冲突"; 收紧重解救回来的配方回放也要达标.
#[test]
fn test_postcheck_residual_does_not_turn_feasible_into_conflict() {
    let request = load(include_str!("fixtures/postcheck_raw_bounds.json"));
    let result = solve(&request);
    assert_ne!(result.reason.as_deref(), Some(CONFLICT));
    assert_recipe_passes_review(&request, &result);
}

/// 收紧重解只收复核没过的行: 某种指标每种煤都正好等于界时, 那一行恒成立
/// (等价于 `Σ0·x ≤ 0`), 收一丝就变成恒不成立, 整份可行的合同会被拖死.
/// 全池岩相取同一个占位值是生产数据的常态.
#[test]
fn test_tightening_skips_rows_binding_by_construction() {
    let request = load(include_str!("fixtures/zero_row_petro.json"));
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    assert_recipe_passes_review(&request, &result);
    let cost = result.cost.as_ref().unwrap().net_per_ton;
    assert!((cost - 2244.72).abs() < 0.01, "成本 {cost}");
}

/// 合同卡在判定边缘时, 解可能靠一种占比不到十万分之一的煤才达标. 输出的配方不能把它截掉:
/// 截掉后照着配方验算, 灰分就判不达标了. 达标所需的零头煤要留在配方与订单里, 配方回放照样达标.
#[test]
fn test_recipe_keeps_the_dust_it_needs() {
    let request = load(include_str!("fixtures/postcheck_truncate.json"));
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    assert!(!result.undetermined);
    assert!(
        result.recipe.values().any(|ratio| *ratio <= 1e-5),
        "达标靠的那份零头应在配方里: {:?}",
        result.recipe
    );
    assert_eq!(result.recipe.len(), result.orders.len());
    assert_recipe_passes_review(&request, &result);
}

/// 不需要的零头照旧不进配方: 门槛只在"截掉会让配方判不达标"时才往下放.
#[test]
fn test_recipe_drops_dust_it_does_not_need() {
    let request = load(include_str!("fixtures/postcheck_raw_bounds.json"));
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    assert!(
        result.recipe.values().all(|ratio| *ratio > 1e-5),
        "{:?}",
        result.recipe
    );
}

/// 按整铲配时, 达标所需的零头煤会被看见: 要么去掉, 要么给足一铲. 结果是真正的整铲配方,
/// 而不是"显示一种煤 100%、实际靠看不见的零头达标".
#[test]
fn test_shovel_gives_needed_dust_a_whole_shovel() {
    let request = load(include_str!("fixtures/dust_needs_one_shovel.json"));
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
    assert_eq!(shovels, 100);
    assert!(result.orders.len() >= 2, "{:?}", result.recipe);
    assert_recipe_passes_review(&request, &result);
}

/// 收紧重解只在复核失败时才走: 正常路径一次求解就该过, 结果与收紧无关.
#[test]
fn test_ordinary_solve_unchanged_by_postcheck_retry() {
    let coals = vec![
        coal_from_tuple(
            "甲煤",
            (0.6, 9.0, 24.0, 85.0, 18.0, 0.1, 65.0, 9.0, 1500.0, 50.0),
        ),
        coal_from_tuple(
            "乙煤",
            (1.4, 11.0, 22.0, 70.0, 14.0, 0.1, 55.0, 10.0, 1200.0, 40.0),
        ),
    ];
    let request = BlendRequest {
        coals,
        specs: vec![Spec::upper("S", 1.0)],
        total_quantity: None,
        truncate_decimal: false,
        fixed_ratios: None,
        rank_interaction: None,
        price_factor: None,
        price_robustness: None,
        shovel_tons: None,
        min_ratios: HashMap::new(),
    };
    let result = solve(&request);
    assert!(result.ok, "{:?}", result.reason);
    // S: 0.6x + 1.4(1−x) = 1.0 ⇒ 甲煤 0.5, 顶在界上.
    assert!(
        (result.recipe["甲煤"] - 0.5).abs() < 1e-6,
        "{:?}",
        result.recipe
    );
    // 正常结果不带"没算出结论"的标记, 输出里也没有这个字段.
    assert!(!result.undetermined);
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("undetermined"));
    let cost = result.cost.unwrap().net_per_ton;
    assert!((cost - 1395.0).abs() < 1e-4, "成本 {cost}");
}

/// 只差一丝的真不可行: 合同界落在判定格点上、执行界是"下一格 − 1e-5"时, 求解器常
/// 返回迭代不收敛而不是不可行证书. 但逐项诊断一旦给出真凶 (子问题最优值确实越过执行
/// 界), 合同不可行就已经被证明, 与求解器状态无关 —— 要报"约束冲突", 不能说"无法判定".
#[test]
fn test_numerical_failure_with_diagnosis_reports_conflict() {
    for fixture in [
        include_str!("fixtures/numerical_max_iterations.json"),
        include_str!("fixtures/min_v_infeasible.json"),
    ] {
        let request = load(fixture);
        let result = solve(&request);
        assert!(!result.ok);
        assert!(!result.undetermined, "证实了不可行就不是没算出结论");
        assert!(
            !result.infeasible_bounds.is_empty(),
            "fixture 应自带真凶诊断: {:?}",
            result.reason
        );
        assert_eq!(
            result.reason.as_deref(),
            Some(CONFLICT),
            "{:?}",
            result.reason
        );
    }
}
