//! 煤阶交互罚项: CSR配 = Σxᵢ·CSRᵢ − k·D, D = 配比加权的煤阶 (Ro) 方差.
//! 表达"单做各 65、双做降到 60"这类配煤不相容; k 默认不传 = 不启用, 结果与现在完全一致.
//!
//! 全部用 fixture, 不读 master. Ro 由挥发换算: Ro ≈ 2.478 − 0.0458·Vdaf.

use blend_kit::*;

fn ro(vdaf: f64) -> f64 {
    2.478 - 0.0458 * vdaf
}

/// 顺序: (S, A, V, G, Y, petro, CSR, M, FOB, FRT).
fn low_rank_gap_pair() -> Vec<Coal> {
    vec![
        coal_from_tuple(
            "低挥发",
            (2.0, 10.0, 18.0, 90.0, 14.0, 0.1, 65.0, 10.0, 1000.0, 0.0),
        ),
        coal_from_tuple(
            "高挥发",
            (0.4, 10.0, 30.0, 60.0, 18.0, 0.1, 65.0, 10.0, 1000.0, 0.0),
        ),
    ]
}

fn request(coals: Vec<Coal>, specs: Vec<Spec>, k: Option<f64>) -> BlendRequest {
    BlendRequest {
        coals,
        specs,
        total_quantity: None,
        truncate_decimal: false,
        fixed_ratios: None,
        rank_interaction: k.map(|k| RankInteraction { k }),
    }
}

fn csr_of(result: &BlendResult) -> f64 {
    result
        .indicator_check
        .iter()
        .find(|check| check.indicator == "CSR")
        .expect("体检里应有 CSR")
        .value
}

/// 按定义独立算一遍真实的 CSR配 (不经求解器).
fn true_csr(result: &BlendResult, coals: &[Coal], k: f64) -> f64 {
    let parts: Vec<(f64, f64, f64)> = coals
        .iter()
        .filter_map(|coal| {
            let x = *result.recipe.get(&coal.name)?;
            Some((x, coal.get("CSR").unwrap(), ro(coal.get("V").unwrap())))
        })
        .collect();
    let base: f64 = parts.iter().map(|(x, csr, _)| x * csr).sum();
    let mean: f64 = parts.iter().map(|(x, _, r)| x * r).sum();
    let var: f64 = parts.iter().map(|(x, _, r)| x * (r - mean).powi(2)).sum();
    base - k * var
}

fn half_half() -> Option<std::collections::HashMap<String, f64>> {
    Some(
        [("低挥发", 1.0), ("高挥发", 1.0)]
            .into_iter()
            .map(|(n, s)| (n.to_string(), s))
            .collect(),
    )
}

/// 朋友的经验: 单做各 65, 1:1 双做 60. k = 5 / (0.25·ΔRo²) ≈ 66.2.
#[test]
fn test_half_half_blend_drops_to_60() {
    let d_ro = ro(18.0) - ro(30.0);
    let k = 5.0 / (0.25 * d_ro * d_ro);
    let mut req = request(low_rank_gap_pair(), vec![Spec::lower("CSR", 50.0)], Some(k));
    req.fixed_ratios = half_half();
    let result = solve(&req);
    assert!(result.ok, "{:?}", result.reason);
    assert!(
        (csr_of(&result) - 60.0).abs() < 1e-9,
        "得 {}",
        csr_of(&result)
    );
    let var = 0.25 * d_ro * d_ro;
    assert!((result.rank_variance.unwrap() - var).abs() < 1e-12);
    assert!((result.csr_interaction_penalty.unwrap() - 5.0).abs() < 1e-9);
}

/// 单煤时方差为 0, 自动回到单煤值.
#[test]
fn test_single_coal_has_no_penalty() {
    let mut req = request(
        low_rank_gap_pair(),
        vec![Spec::lower("CSR", 50.0)],
        Some(66.0),
    );
    req.fixed_ratios = Some([("低挥发".to_string(), 1.0)].into_iter().collect());
    let result = solve(&req);
    assert!((csr_of(&result) - 65.0).abs() < 1e-9);
    assert!(result.csr_interaction_penalty.unwrap().abs() < 1e-12);
}

/// 不传 k 或 k = 0: 结果与现在逐字段相同 —— 不打扰现有工作流.
#[test]
fn test_absent_or_zero_k_changes_nothing() {
    let specs = vec![Spec::upper("S", 1.2), Spec::lower("CSR", 63.0)];
    let plain =
        serde_json::to_value(solve(&request(low_rank_gap_pair(), specs.clone(), None))).unwrap();
    let zero =
        serde_json::to_value(solve(&request(low_rank_gap_pair(), specs, Some(0.0)))).unwrap();
    assert_eq!(plain, zero);
    assert!(plain.get("rank_variance").is_none(), "未启用时不输出新字段");
}

/// 求最优: S ≤ 1.2 逼低挥发煤 ≤ 50%, G ≥ 75 逼它 ≥ 50%, 不计交互只能 1:1 (CSR 65 ≥ 63);
/// 计交互后 1:1 只有 60,
/// 必须换进中间煤阶的贵煤. 结果的 CSR 必须是真实值且达标, 成本不低于不计交互时.
#[test]
fn test_optimizer_avoids_incompatible_pair() {
    let mut coals = low_rank_gap_pair();
    coals.push(coal_from_tuple(
        "中间煤阶",
        (1.2, 10.0, 24.0, 85.0, 16.0, 0.1, 65.0, 10.0, 1500.0, 0.0),
    ));
    let d_ro = ro(18.0) - ro(30.0);
    let k = 5.0 / (0.25 * d_ro * d_ro);
    let specs = vec![
        Spec::upper("S", 1.2),
        Spec::lower("G", 75.0),
        Spec::lower("CSR", 63.0),
    ];
    let plain = solve(&request(coals.clone(), specs.clone(), None));
    assert!(
        (plain.recipe["低挥发"] - 0.5).abs() < 1e-4,
        "前提: 不计交互是 1:1, 得 {:?}",
        plain.recipe
    );
    let with_k = solve(&request(coals.clone(), specs, Some(k)));
    assert!(plain.ok && with_k.ok, "{:?}", with_k.reason);

    let reported = csr_of(&with_k);
    let truth = true_csr(&with_k, &coals, k);
    assert!(
        (reported - truth).abs() < 1e-6,
        "报告 {reported} 与真实 {truth} 不符"
    );
    assert!(truth >= 63.0 - 1e-6, "真实 CSR {truth} 未达标");
    assert!(
        with_k.recipe.contains_key("中间煤阶"),
        "{:?}",
        with_k.recipe
    );
    let cost = |r: &BlendResult| r.cost.as_ref().unwrap().cif_per_ton;
    assert!(cost(&with_k) >= cost(&plain) - 1e-6);
}

/// 不计交互都配不出来, 计了只会更难: 照样报不可行.
#[test]
fn test_infeasible_stays_infeasible() {
    let specs = vec![Spec::lower("CSR", 70.0)];
    let result = solve(&request(low_rank_gap_pair(), specs, Some(66.0)));
    assert!(!result.ok);
}

/// 非法 k 不让求解失败: 按未启用处理, 加一条警告.
#[test]
fn test_invalid_k_is_ignored_with_warning() {
    let specs = vec![Spec::upper("S", 1.2), Spec::lower("CSR", 63.0)];
    let plain = solve(&request(low_rank_gap_pair(), specs.clone(), None));
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        let result = solve(&request(low_rank_gap_pair(), specs.clone(), Some(bad)));
        assert_eq!(result.recipe, plain.recipe);
        assert!(result.rank_variance.is_none());
        assert!(
            result.warnings.iter().any(|w| w.contains("煤阶交互")),
            "{:?}",
            result.warnings
        );
    }
}

/// 有煤既没挥发也没反射率, 算不出煤阶: 不启用, 加警告, 结果照常.
#[test]
fn test_missing_rank_input_skips_with_warning() {
    let mut coals = low_rank_gap_pair();
    coals[0].props.remove("V");
    let result = solve(&request(coals, vec![Spec::lower("G", 50.0)], Some(66.0)));
    assert!(result.ok);
    assert!(result.rank_variance.is_none());
    assert!(
        result.warnings.iter().any(|w| w.contains("低挥发")),
        "{:?}",
        result.warnings
    );
}

/// 有反射率时用反射率均值, 不用挥发换算.
#[test]
fn test_petrography_mean_preferred_over_vdaf() {
    let mut coals = low_rank_gap_pair();
    for (coal, mean) in coals.iter_mut().zip([1.30, 1.10]) {
        coal.petrography = Some(Petrography {
            hist: Vec::new(),
            vitrinite_pct: 70.0,
            mean: Some(mean),
            std_dev: Some(0.06),
        });
    }
    let mut req = request(coals, vec![Spec::lower("CSR", 50.0)], Some(100.0));
    req.fixed_ratios = half_half();
    let result = solve(&req);
    let var = 0.25 * 0.2_f64.powi(2);
    assert!((result.rank_variance.unwrap() - var).abs() < 1e-12);
}
