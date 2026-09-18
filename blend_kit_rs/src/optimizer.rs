//! 混合配煤求解器.
//!
//! Clarabel LP 负责生成最低成本候选配方；每个指标由独立公式评估。G/CSR 可
//! 显式接收已训练且通过门控的评估器，岩相在 LP 后按全方差定律复验并收紧重算。

use crate::model::*;
use crate::petrography::{self, Petrography, NOTCH_WINDOW};
use crate::predict::EvaluatorSet;
use crate::quality::{
    acceptance_rule, check_value, effective_lower, effective_upper, formula_for, validate_request,
    MetricFormula,
};
use clarabel::algebra::CscMatrix;
use clarabel::solver::*;
use std::collections::{HashMap, HashSet};

const MAX_EVALUATION_ITERATIONS: usize = 4;
const PETRO_SHRINK_SAFETY: f64 = 0.999;
const SOLUTION_TOLERANCE: f64 = 1e-8;
const OUTPUT_RATIO_TOLERANCE: f64 = 1e-5;

/// 基础求解入口，不读取训练样本，也不在求解期间拟合模型.
pub fn solve(request: &BlendRequest) -> BlendResult {
    solve_with_evaluators(request, &EvaluatorSet::default())
}

/// 使用调用方预先训练好的可选评估器求解.
///
/// `ok` 只表示是否得到配方；可信度由 `quality_status` 表示。
pub fn solve_with_evaluators(request: &BlendRequest, evaluators: &EvaluatorSet) -> BlendResult {
    if let Err(reason) = validate_request(request) {
        return BlendResult::infeasible(&reason, Vec::new());
    }

    let mut warnings = evaluators.warnings.clone();
    if evaluators.csr.is_some() {
        for coal in &request.coals {
            let missing: Vec<&str> = ["S", "A", "V", "G", "Y", "M"]
                .into_iter()
                .filter(|indicator| !coal.has(indicator))
                .collect();
            if !missing.is_empty() {
                warnings.push(format!(
                    "{}: 缺输入指标 {}，本次 CSR 回退录入代理值",
                    coal.name,
                    missing
                        .into_iter()
                        .map(label_zh)
                        .collect::<Vec<_>>()
                        .join("/")
                ));
            }
        }
    }
    let active_specs: Vec<&Spec> = request.specs.iter().filter(|spec| spec.enabled).collect();
    let mut coals = request.coals.clone();

    // 有 μ/σ 或直方图时，用单煤真实 σ 补齐 LP 代理字段。
    for coal in &mut coals {
        if !coal.has("petro") {
            if let Some((_, std_dev)) = coal.petrography.as_ref().and_then(Petrography::mean_std) {
                coal.props.insert("petro".into(), std_dev);
            }
        }
    }

    // 只有 Hard 约束会剔除缺字段煤；Soft/Advisory 不能改变候选煤池。
    let mut required: HashSet<String> = active_specs
        .iter()
        .filter(|spec| spec.enforcement == Enforcement::Hard)
        .map(|spec| spec.indicator.clone())
        .collect();
    if evaluators.csr.is_some() && required.remove("CSR") {
        required.extend(["S", "A", "V", "G", "Y", "M"].into_iter().map(String::from));
    }

    let mut kept = Vec::new();
    for coal in &coals {
        let missing: Vec<&String> = required
            .iter()
            .filter(|indicator| !coal.has(indicator))
            .collect();
        if missing.is_empty() {
            kept.push(coal);
        } else {
            warnings.push(format!(
                "剔除 {}: 缺指标 {}",
                coal.name,
                missing
                    .iter()
                    .map(|indicator| label_zh(indicator))
                    .collect::<Vec<_>>()
                    .join("/")
            ));
        }
    }
    if kept.is_empty() {
        return BlendResult::infeasible("无可用煤", warnings);
    }

    let formulas = build_formulas(&kept, evaluators);
    if let Some(spec) = active_specs
        .iter()
        .find(|spec| {
            spec.enforcement == Enforcement::Hard && !formulas.contains_key(&spec.indicator)
        })
        .copied()
    {
        let reason = format!("{}缺少可用输入", label_zh(&spec.indicator));
        return BlendResult::infeasible(&reason, warnings);
    }
    let petro_spec = active_specs
        .iter()
        .find(|spec| spec.indicator == "petro")
        .copied();
    let petro_internal_upper = petro_spec
        .filter(|spec| {
            spec.enforcement == Enforcement::Hard
                && matches!(spec.direction, Direction::Upper | Direction::Range)
        })
        .and_then(|spec| {
            let rule = acceptance_rule(spec, request.truncate_decimal);
            spec.max
                .map(|maximum| effective_upper(maximum, &rule) - spec.margin.unwrap_or(0.0))
        });
    let petro_internal_lower = petro_spec
        .filter(|spec| {
            spec.enforcement == Enforcement::Hard
                && matches!(spec.direction, Direction::Lower | Direction::Range)
        })
        .and_then(|spec| {
            let rule = acceptance_rule(spec, request.truncate_decimal);
            spec.min
                .map(|minimum| effective_lower(minimum, &rule) + spec.margin.unwrap_or(0.0))
        });

    let mut petro_proxy_upper = None;
    let mut petro_proxy_lower = None;
    let mut iteration = 0;
    let mut fallback: Option<BlendResult> = None;

    loop {
        let Some((mut result, ratios)) = solve_once(
            &kept,
            &active_specs,
            request,
            &formulas,
            evaluators,
            petro_proxy_lower,
            petro_proxy_upper,
            warnings.clone(),
        ) else {
            return if let Some(previous) = fallback {
                if petro_spec.is_some_and(|spec| spec.enforcement == Enforcement::Hard) {
                    let mut failure_warnings = previous.warnings;
                    failure_warnings
                        .push("岩相精确校验修复后 LP 不可行；请调整煤池或合同级别".into());
                    BlendResult::infeasible("岩相精确 Hard 复核未找到可行配方", failure_warnings)
                } else {
                    let mut previous = previous;
                    previous
                        .warnings
                        .push("岩相修复后 LP 不可行，已保留候选方案".into());
                    finalize_quality_status(&mut previous, &active_specs);
                    previous
                }
            } else {
                BlendResult::infeasible("约束冲突, LP 不可行", warnings)
            };
        };
        result.evaluation_iterations = iteration;

        let participating: Vec<(&Coal, f64)> = kept
            .iter()
            .zip(&ratios)
            .filter(|(_, ratio)| **ratio > OUTPUT_RATIO_TOLERANCE)
            .map(|(coal, ratio)| (*coal, *ratio))
            .collect();
        let petro_parts: Vec<(&Petrography, f64)> = participating
            .iter()
            .filter_map(|(coal, ratio)| {
                coal.petrography
                    .as_ref()
                    .filter(|data| data.is_valid())
                    .map(|data| (data, *ratio))
            })
            .collect();

        if petro_parts.len() != participating.len() {
            let any_petro_input = participating
                .iter()
                .any(|(coal, _)| coal.petrography.is_some());
            if petro_spec.is_some() && any_petro_input {
                let missing: Vec<&str> = participating
                    .iter()
                    .filter(|(coal, _)| {
                        !coal.petrography.as_ref().is_some_and(Petrography::is_valid)
                    })
                    .map(|(coal, _)| coal.name.as_str())
                    .collect();
                result.warnings.push(format!(
                    "岩相精确复核跳过: {} 缺有效煤岩输入",
                    missing.join("/")
                ));
            }
            finalize_quality_status(&mut result, &active_specs);
            return result;
        }

        let Some((mean, sigma)) = petrography::mix_mean_std(&petro_parts) else {
            if petro_spec.is_some() {
                result
                    .warnings
                    .push("岩相精确复核跳过: μ/σ 输入无法计算".into());
            }
            finalize_quality_status(&mut result, &active_specs);
            return result;
        };

        let all_histograms = petro_parts.iter().all(|(data, _)| data.has_histogram());
        let mixed_histogram = all_histograms
            .then(|| petrography::mix_histogram(&petro_parts))
            .flatten();
        let notch = mixed_histogram.as_deref().and_then(|histogram| {
            petrography::detect_notch(histogram, NOTCH_WINDOW.0, NOTCH_WINDOW.1)
        });
        if let Some(notch_data) = &notch {
            result.warnings.push(format!(
                "岩相: {:.1}~{:.1} 主焦区间存在凹口（谷深比 {:.2}）",
                NOTCH_WINDOW.0, NOTCH_WINDOW.1, notch_data.depth_ratio
            ));
        }

        let petro_outcome = check_value(sigma, petro_spec, request.truncate_decimal, true);
        update_petro_check(
            &mut result,
            sigma,
            &petro_outcome,
            if all_histograms {
                EvaluationMethod::Histogram
            } else {
                EvaluationMethod::Moments
            },
        );
        result.petrography_check = Some(PetrographyCheck {
            mean,
            sigma,
            sigma_max: petro_spec.and_then(|spec| spec.max),
            sigma_ok: petro_spec.map(|_| petro_outcome.status != EvaluationStatus::Fail),
            notch,
            refine_iterations: iteration,
        });

        let above_upper =
            petro_internal_upper.is_some_and(|limit| sigma > limit + SOLUTION_TOLERANCE);
        let below_lower =
            petro_internal_lower.is_some_and(|limit| sigma + SOLUTION_TOLERANCE < limit);
        let needs_refinement = above_upper || below_lower;
        if needs_refinement && iteration < MAX_EVALUATION_ITERATIONS {
            if let Some(limit) = petro_internal_upper.filter(|_| above_upper) {
                let current = petro_proxy_upper.unwrap_or(limit);
                petro_proxy_upper = Some(current * (limit / sigma).powi(2) * PETRO_SHRINK_SAFETY);
            }
            if let Some(limit) = petro_internal_lower.filter(|_| below_lower) {
                let current = petro_proxy_lower.unwrap_or(limit);
                let safe_sigma = sigma.max(SOLUTION_TOLERANCE);
                petro_proxy_lower =
                    Some(current * (limit / safe_sigma).powi(2) / PETRO_SHRINK_SAFETY);
            }
            iteration += 1;
            fallback = Some(result);
            continue;
        }
        if needs_refinement {
            result.warnings.push(format!(
                "岩相精确值 σ={sigma:.3} 未达到内部保护区间，启发式修复 {iteration} 轮后停止"
            ));
            if petro_spec.is_some_and(|spec| spec.enforcement == Enforcement::Hard) {
                return BlendResult::infeasible(
                    "岩相精确 Hard 复核未找到可行配方",
                    result.warnings,
                );
            }
        }
        if petro_outcome.status == EvaluationStatus::Fail
            && petro_spec.is_some_and(|spec| spec.enforcement == Enforcement::Hard)
        {
            result
                .warnings
                .push(format!("岩相精确值 σ={sigma:.3} 未通过合同 Hard 判定"));
            return BlendResult::infeasible("岩相精确 Hard 复核未找到可行配方", result.warnings);
        }
        finalize_quality_status(&mut result, &active_specs);
        return result;
    }
}

fn build_formulas(coals: &[&Coal], models: &EvaluatorSet) -> HashMap<String, MetricFormula> {
    let mut formulas = HashMap::new();
    for indicator in INDICATORS {
        if let Some(formula) = formula_for(indicator, coals) {
            formulas.insert(indicator.into(), formula);
        }
    }

    if let (Some(model), Some(base)) = (models.g.as_ref(), formulas.get("G").cloned()) {
        let coefficients: Vec<f64> = base
            .proxy_coefficients
            .iter()
            .map(|value| model.predictor.slope * value)
            .collect();
        formulas.insert(
            "G".into(),
            MetricFormula {
                proxy_coefficients: base.proxy_coefficients,
                numerators: coefficients,
                denominators: vec![1.0; coals.len()],
                intercept: model.predictor.intercept,
                method: EvaluationMethod::AffineCalibration,
                verified: true,
                uncertainty: Some(model.p90_abs_error),
                model: Some(model.summary(true)),
            },
        );
    }

    if let Some(model) = &models.csr {
        let predictor = &model.predictor;
        let evaluated_coefficients: Option<Vec<f64>> = coals
            .iter()
            .map(|coal| {
                Some(
                    predictor.beta_s * coal.get("S")?
                        + predictor.beta_a * coal.get("A")?
                        + predictor.beta_v * coal.get("V")?
                        + predictor.beta_g * coal.get("G")?
                        + predictor.beta_y * coal.get("Y")?
                        + predictor.beta_m * coal.get("M")?,
                )
            })
            .collect();
        if let Some(evaluated_coefficients) = evaluated_coefficients {
            let proxy_coefficients: Vec<f64> = coals
                .iter()
                .zip(&evaluated_coefficients)
                .map(|(coal, evaluated)| coal.get("CSR").unwrap_or(*evaluated))
                .collect();
            formulas.insert(
                "CSR".into(),
                MetricFormula {
                    proxy_coefficients,
                    numerators: evaluated_coefficients,
                    denominators: vec![1.0; coals.len()],
                    intercept: predictor.intercept,
                    method: EvaluationMethod::Regression,
                    verified: true,
                    uncertainty: Some(model.p90_abs_error),
                    model: Some(model.summary(true)),
                },
            );
        }
    }
    formulas
}

fn expanded_domain(minimum: f64, maximum: f64, ratio: f64) -> (f64, f64) {
    let width = (maximum - minimum).max(1e-9);
    (minimum - width * ratio, maximum + width * ratio)
}

fn push_linear_domain(
    coefficients: Vec<f64>,
    intercept: f64,
    minimum: f64,
    maximum: f64,
    inequalities: &mut Vec<Vec<f64>>,
    bounds: &mut Vec<f64>,
) {
    inequalities.push(coefficients.clone());
    bounds.push(maximum - intercept);
    inequalities.push(coefficients.into_iter().map(|value| -value).collect());
    bounds.push(intercept - minimum);
}

/// 已验证模型用于 Hard 约束时，训练域本身也必须进入 LP。这样求解器不能
/// 通过选择便宜的域外配方来利用外推值“满足”合同。
fn append_hard_model_domains(
    coals: &[&Coal],
    specs: &[&Spec],
    models: &EvaluatorSet,
    inequalities: &mut Vec<Vec<f64>>,
    bounds: &mut Vec<f64>,
) -> Option<()> {
    let hard_g = specs
        .iter()
        .any(|spec| spec.indicator == "G" && spec.enforcement == Enforcement::Hard);
    let hard_csr = specs
        .iter()
        .any(|spec| spec.indicator == "CSR" && spec.enforcement == Enforcement::Hard);

    if hard_g && models.g.is_some() {
        let model = models.g.as_ref()?;
        let coefficients = coals
            .iter()
            .map(|coal| coal.get("G"))
            .collect::<Option<Vec<_>>>()?;
        let (minimum, maximum) = expanded_domain(
            model.training_min,
            model.training_max,
            models.extrapolation_ratio,
        );
        push_linear_domain(coefficients, 0.0, minimum, maximum, inequalities, bounds);
    }

    if hard_csr {
        if let Some(model) = &models.csr {
            for (index, indicator) in ["S", "A", "V", "G", "Y", "M"].into_iter().enumerate() {
                let coefficients = coals
                    .iter()
                    .map(|coal| coal.get(indicator))
                    .collect::<Option<Vec<_>>>()?;
                let (minimum, maximum) = expanded_domain(
                    model.training_min[index],
                    model.training_max[index],
                    models.extrapolation_ratio,
                );
                push_linear_domain(coefficients, 0.0, minimum, maximum, inequalities, bounds);
            }
        }
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn solve_once(
    coals: &[&Coal],
    specs: &[&Spec],
    request: &BlendRequest,
    formulas: &HashMap<String, MetricFormula>,
    models: &EvaluatorSet,
    petro_proxy_lower: Option<f64>,
    petro_proxy_upper: Option<f64>,
    mut warnings: Vec<String>,
) -> Option<(BlendResult, Vec<f64>)> {
    let count = coals.len();
    let costs: Vec<f64> = coals.iter().map(|coal| coal.cif()).collect();
    let mut inequalities = Vec::new();
    let mut bounds = Vec::new();

    for spec in specs
        .iter()
        .filter(|spec| spec.enforcement == Enforcement::Hard)
    {
        let formula = formulas.get(&spec.indicator)?;
        let rule = acceptance_rule(spec, request.truncate_decimal);
        let margin = spec.margin.unwrap_or(0.0);
        if matches!(spec.direction, Direction::Upper | Direction::Range) {
            if let Some(maximum) = spec.max {
                let mut upper = effective_upper(maximum, &rule) - margin;
                if spec.indicator == "petro" {
                    upper = petro_proxy_upper.unwrap_or(upper);
                }
                let (row, bound) = formula.upper_constraint(upper);
                inequalities.push(row);
                bounds.push(bound);
            }
        }
        if matches!(spec.direction, Direction::Lower | Direction::Range) {
            if let Some(minimum) = spec.min {
                let mut lower = effective_lower(minimum, &rule) + margin;
                if spec.indicator == "petro" {
                    lower = petro_proxy_lower.unwrap_or(lower);
                }
                let (row, bound) = formula.lower_constraint(lower);
                inequalities.push(row);
                bounds.push(bound);
            }
        }
    }
    append_hard_model_domains(coals, specs, models, &mut inequalities, &mut bounds)?;

    let problem = LpProblem {
        n: count,
        c: costs,
        a_ub: inequalities,
        b_ub: bounds,
    };
    let (ratios, _) = problem.solve()?;

    let recipe = coals
        .iter()
        .zip(&ratios)
        .filter(|(_, ratio)| **ratio > OUTPUT_RATIO_TOLERANCE)
        .map(|(coal, ratio)| (coal.name.clone(), *ratio))
        .collect();
    let fob_per_ton: f64 = coals
        .iter()
        .zip(&ratios)
        .map(|(coal, ratio)| coal.fob * ratio)
        .sum();
    let frt_per_ton: f64 = coals
        .iter()
        .zip(&ratios)
        .map(|(coal, ratio)| coal.frt * ratio)
        .sum();
    let cif_per_ton = fob_per_ton + frt_per_ton;
    let cost = CostBreakdown {
        fob_per_ton,
        frt_per_ton,
        cif_per_ton,
        total_fob: request
            .total_quantity
            .map(|quantity| quantity * fob_per_ton),
        total_frt: request
            .total_quantity
            .map(|quantity| quantity * frt_per_ton),
        total_cif: request
            .total_quantity
            .map(|quantity| quantity * cif_per_ton),
        purchase_adjust_per_ton: 0.0,
        penalty_per_ton: 0.0,
        net_per_ton: cif_per_ton,
        total_purchase_adjust: request.total_quantity.map(|_| 0.0),
        total_penalty: request.total_quantity.map(|_| 0.0),
        total_net: request
            .total_quantity
            .map(|quantity| quantity * cif_per_ton),
    };
    let mut orders: Vec<OrderItem> = coals
        .iter()
        .zip(&ratios)
        .filter(|(_, ratio)| **ratio > OUTPUT_RATIO_TOLERANCE)
        .map(|(coal, ratio)| {
            let tons = request.total_quantity.map(|quantity| quantity * ratio);
            OrderItem {
                coal: coal.name.clone(),
                ratio: *ratio,
                tons,
                fob_amount: tons.map(|value| value * coal.fob),
                frt_amount: tons.map(|value| value * coal.frt),
                cif_amount: tons.map(|value| value * coal.cif()),
                cif_eff: coal.cif(),
            }
        })
        .collect();
    orders.sort_by(|left, right| right.ratio.total_cmp(&left.ratio));

    let mut indicator_check = Vec::new();
    for indicator in INDICATORS {
        let spec = specs
            .iter()
            .find(|spec| spec.indicator == indicator)
            .copied();
        let Some(formula) = formulas.get(indicator) else {
            if let Some(spec) = spec {
                let proxy_coefficients = coals
                    .iter()
                    .map(|coal| coal.get(indicator))
                    .collect::<Option<Vec<_>>>();
                let proxy = proxy_coefficients
                    .as_deref()
                    .map(|values| values.iter().zip(&ratios).map(|(a, b)| a * b).sum());
                indicator_check.push(IndicatorCheck {
                    indicator: indicator.into(),
                    label_zh: label_zh(indicator).into(),
                    value: proxy.unwrap_or(0.0),
                    min: spec.min,
                    max: spec.max,
                    slack: None,
                    binding: false,
                    proxy_value: proxy,
                    evaluated_value: None,
                    judged_value: None,
                    uncertainty: None,
                    method: EvaluationMethod::Unavailable,
                    status: EvaluationStatus::Unverified,
                    model: None,
                    penalty_per_ton: None,
                });
            }
            continue;
        };
        let Some(evaluated) = formula.evaluate(&ratios) else {
            continue;
        };
        let proxy = formula.proxy_value(&ratios);
        let (mut verified, model_summary) =
            runtime_model_state(indicator, formula, coals, &ratios, models);
        let invalid_model_output = model_summary.is_some()
            && matches!(indicator, "G" | "CSR")
            && !(0.0..=100.0).contains(&evaluated);
        if invalid_model_output {
            verified = false;
            warnings.push(format!(
                "{}模型输出 {evaluated:.3} 超出 0~100 物理范围",
                label_zh(indicator)
            ));
        }
        let mut outcome = check_value(evaluated, spec, request.truncate_decimal, verified);
        if invalid_model_output {
            outcome.status = EvaluationStatus::Fail;
        }
        if model_summary
            .as_ref()
            .is_some_and(|summary| !summary.in_domain)
        {
            warnings.push(format!(
                "{}模型超出训练域，本次只展示为未验证估算",
                label_zh(indicator)
            ));
        }
        indicator_check.push(IndicatorCheck {
            indicator: indicator.into(),
            label_zh: label_zh(indicator).into(),
            value: evaluated,
            min: spec.and_then(|item| item.min),
            max: spec.and_then(|item| item.max),
            slack: outcome.slack,
            binding: outcome.binding,
            proxy_value: Some(proxy),
            evaluated_value: Some(evaluated),
            judged_value: Some(outcome.judged),
            uncertainty: formula.uncertainty,
            method: formula.method,
            status: outcome.status,
            model: model_summary,
            penalty_per_ton: None,
        });
    }

    if specs
        .iter()
        .filter(|spec| spec.enforcement == Enforcement::Hard)
        .any(|spec| {
            indicator_check.iter().any(|check| {
                check.indicator == spec.indicator && check.status == EvaluationStatus::Fail
            })
        })
    {
        return None;
    }

    let mut result = BlendResult {
        ok: true,
        reason: None,
        recipe,
        cost: Some(cost),
        orders,
        indicator_check,
        petrography_check: None,
        warnings,
        quality_status: QualityStatus::Estimated,
        evaluation_iterations: 0,
    };
    finalize_quality_status(&mut result, specs);
    Some((result, ratios))
}

fn runtime_model_state(
    indicator: &str,
    formula: &MetricFormula,
    coals: &[&Coal],
    ratios: &[f64],
    models: &EvaluatorSet,
) -> (bool, Option<ModelSummary>) {
    match indicator {
        "G" => {
            if let Some(model) = &models.g {
                let in_domain =
                    model.is_in_domain(formula.proxy_value(ratios), models.extrapolation_ratio);
                return (
                    formula.verified && in_domain,
                    Some(model.summary(in_domain)),
                );
            }
        }
        "CSR" => {
            if let Some(model) = &models.csr {
                let features = mixed_csr_features(coals, ratios);
                let in_domain = features
                    .is_some_and(|values| model.is_in_domain(values, models.extrapolation_ratio));
                return (
                    formula.verified && in_domain,
                    Some(model.summary(in_domain)),
                );
            }
        }
        _ => {}
    }
    (formula.verified, formula.model.clone())
}

fn mixed_csr_features(coals: &[&Coal], ratios: &[f64]) -> Option<[f64; 6]> {
    let mixed = |indicator: &str| -> Option<f64> {
        coals
            .iter()
            .zip(ratios)
            .map(|(coal, ratio)| Some(coal.get(indicator)? * ratio))
            .sum()
    };
    Some([
        mixed("S")?,
        mixed("A")?,
        mixed("V")?,
        mixed("G")?,
        mixed("Y")?,
        mixed("M")?,
    ])
}

fn update_petro_check(
    result: &mut BlendResult,
    sigma: f64,
    outcome: &crate::quality::CheckOutcome,
    method: EvaluationMethod,
) {
    if let Some(check) = result
        .indicator_check
        .iter_mut()
        .find(|check| check.indicator == "petro")
    {
        check.value = sigma;
        check.evaluated_value = Some(sigma);
        check.judged_value = Some(outcome.judged);
        check.slack = outcome.slack;
        check.binding = outcome.binding;
        check.method = method;
        check.status = outcome.status;
        check.uncertainty = None;
        check.model = None;
    }
}

fn finalize_quality_status(result: &mut BlendResult, specs: &[&Spec]) {
    if !result.ok {
        result.quality_status = QualityStatus::NeedsReview;
        return;
    }
    let mut estimated = specs.is_empty();
    let mut evaluated_contract_items = 0usize;
    for spec in specs {
        if spec.enforcement == Enforcement::Advisory {
            continue;
        }
        evaluated_contract_items += 1;
        let Some(check) = result
            .indicator_check
            .iter()
            .find(|check| check.indicator == spec.indicator)
        else {
            result.quality_status = QualityStatus::NeedsReview;
            return;
        };
        match check.status {
            EvaluationStatus::Fail => {
                result.quality_status = QualityStatus::NeedsReview;
                return;
            }
            EvaluationStatus::Unverified => estimated = true,
            EvaluationStatus::Pass | EvaluationStatus::TolerancePass => {}
        }
    }
    if evaluated_contract_items == 0 {
        result.quality_status = QualityStatus::Estimated;
        return;
    }
    result.quality_status = if estimated {
        QualityStatus::Estimated
    } else {
        QualityStatus::Verified
    };
}

// ============================================================================
// Clarabel LP 子问题
// ============================================================================

struct LpProblem {
    n: usize,
    c: Vec<f64>,
    a_ub: Vec<Vec<f64>>,
    b_ub: Vec<f64>,
}

impl LpProblem {
    fn solve(&self) -> Option<(Vec<f64>, f64)> {
        let n = self.n;
        let inequality_count = self.a_ub.len();
        let total_rows = 1 + inequality_count + n;
        let mut triplets = Vec::new();

        for column in 0..n {
            triplets.push((0, column, 1.0));
        }
        for (row_index, row) in self.a_ub.iter().enumerate() {
            for (column, value) in row.iter().enumerate() {
                triplets.push((1 + row_index, column, *value));
            }
        }
        for column in 0..n {
            triplets.push((1 + inequality_count + column, column, -1.0));
        }

        let constraint_matrix = build_csc(total_rows, n, &triplets);
        let mut right_hand_side = vec![1.0];
        right_hand_side.extend_from_slice(&self.b_ub);
        right_hand_side.extend(std::iter::repeat_n(0.0, n));
        let quadratic = CscMatrix::<f64>::zeros((n, n));
        let cones = [ZeroConeT(1), NonnegativeConeT(inequality_count + n)];
        let settings = DefaultSettingsBuilder::<f64>::default()
            .verbose(false)
            .max_iter(200)
            .build()
            .ok()?;
        let mut solver = DefaultSolver::new(
            &quadratic,
            &self.c,
            &constraint_matrix,
            &right_hand_side,
            &cones,
            settings,
        );
        solver.solve();
        if !matches!(
            solver.solution.status,
            SolverStatus::Solved | SolverStatus::AlmostSolved
        ) {
            return None;
        }
        let mut solution = solver.solution.x.clone();
        let raw_sum: f64 = solution.iter().sum();
        let raw_valid = solution
            .iter()
            .all(|value| value.is_finite() && *value >= -SOLUTION_TOLERANCE)
            && (raw_sum - 1.0).abs() <= SOLUTION_TOLERANCE;
        if !raw_valid {
            return None;
        }
        for value in &mut solution {
            *value = value.max(0.0);
        }
        let normalized_sum: f64 = solution.iter().sum();
        if !normalized_sum.is_finite() || normalized_sum <= 0.0 {
            return None;
        }
        for value in &mut solution {
            *value /= normalized_sum;
        }
        let valid = self.a_ub.iter().zip(&self.b_ub).all(|(row, bound)| {
            row.iter()
                .zip(&solution)
                .map(|(coefficient, value)| coefficient * value)
                .sum::<f64>()
                <= bound + SOLUTION_TOLERANCE
        });
        let objective = self
            .c
            .iter()
            .zip(&solution)
            .map(|(coefficient, value)| coefficient * value)
            .sum();
        valid.then_some((solution, objective))
    }
}

fn build_csc(rows: usize, columns: usize, triplets: &[(usize, usize, f64)]) -> CscMatrix<f64> {
    let mut by_column: Vec<Vec<(usize, f64)>> = vec![Vec::new(); columns];
    for &(row, column, value) in triplets {
        by_column[column].push((row, value));
    }
    let mut column_pointers = Vec::with_capacity(columns + 1);
    let mut row_values = Vec::new();
    let mut nonzero_values = Vec::new();
    column_pointers.push(0);
    for column in &mut by_column {
        column.sort_by_key(|&(row, _)| row);
        for &(row, value) in column.iter() {
            row_values.push(row);
            nonzero_values.push(value);
        }
        column_pointers.push(row_values.len());
    }
    CscMatrix::new(rows, columns, column_pointers, row_values, nonzero_values)
}
