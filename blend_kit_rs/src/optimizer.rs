//! 混合配煤求解器.
//!
//! Clarabel LP 负责生成最低成本候选配方；每个指标由独立公式评估。G/CSR 可
//! 显式接收已训练且通过门控的评估器，岩相在 LP 后按全方差定律复验并收紧重算。

use crate::model::*;
use crate::penalty::{cif_eff, cif_eff_or_quoted, clauses_missing_assay};
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
/// 可行性复核的相对容限. 比 SOLUTION_TOLERANCE 宽, 因为档位列不经归一化重投影,
/// 残差保持在 Clarabel 的原始收敛量级 (~1e-8..1e-7).
///
/// 判据是 `activity <= bound + 本值 * (1 + magnitude)`, 其中
/// `magnitude = Σ|aᵢxᵢ|` 再与 `|bound|` 取大. 它**对所有行生效**, 不止计价行.
///
/// 曾经写着"纯 Hard 配方残差本就在 1e-13 量级, 用不满这点容限" —— 这句是错的,
/// 只对 Σx=1 那一行成立 (配比列经归一化重投影, 残差才塌到 1e-13). 指标约束行不
/// 重投影, 残差就是 Clarabel 的原始收敛量级: 45.0 万条 Hard 行样本里最大
/// residual/(1+magnitude) 实测到 9.77e-8, 已经贴着本值。Hard 行同样吃满这点容限。
///
/// 取值依据 (**已被下面那条取代, 保留是为了说明那个 16 倍是怎么来的**): 92 组良态
/// 计价输入 (合同上限 10.0, 单档 rate 10, ash 10.0~13.0 × reject 11.0~15.0) 实测
/// 156 行, 最大 residual/(1+magnitude) = 6.36e-9, 号称留出约 16 倍余量.
/// 那只是一张**窄网格**: 只有计价行、只有一个煤种、界只动了 ash 一项.
///
/// ⚠ **真实余量是 1.01 倍**. 宽网格 (master 4+31 煤池 + 线上 4 煤池 × 五种合同变体
/// × 逐界细扫) 实测最大 residual/(1+magnitude): 线性行 **9.771e-8**, 仿射/回归行
/// **9.882e-8** —— 已经贴着本值的 98.8%.
///
/// 越过这条线的解由 LP 自己判不可行. 也就是说, 解后复核对齐之后,"可行合同报不可行"
/// 的风险整体**转移到了本常量上**: 残差再漂 1.2% 就复发, 只是发作点从体检挪到了 LP
/// 自己. 煤池变宽、合同变紧、Clarabel 升级, 任何一样都可能吃掉这点余量.
///
/// 这两个数由 `measurements::measure_tolerance_headroom` 产出, 可复跑:
/// `cargo test --release -- --ignored measure_tolerance_headroom --nocapture`.
/// 想调本值就先重跑它, 别拿上面那 156 行说事.
///
/// 安全边界: 放行量是 `本值 × (1 + magnitude)`, 随行量级变化 —— 灰分行约 3e-7,
/// 宽煤池上的 CSR/G 行可达约 2.6e-6 (指标单位). 即便按后者算, 仍比化验 0.01%
/// 的分辨率细 4 个数量级, 拒收线仍是硬墙.
///
/// 解后复核 (`quality::ACCEPT_TOLERANCE`) 共用本值, 这是刻意的: 两边判的是同一件
/// 事, 各写一个数就会重演"LP 认、复核不认"那个缺陷.
pub(crate) const FEASIBILITY_TOLERANCE: f64 = 1e-7;
const OUTPUT_RATIO_TOLERANCE: f64 = 1e-5;

/// 基础求解入口，不读取训练样本，也不在求解期间拟合模型.
pub fn solve(request: &BlendRequest) -> BlendResult {
    solve_with_evaluators(request, &EvaluatorSet::default())
}

/// 使用调用方预先训练好的可选评估器求解.
///
/// `ok` 只表示是否得到配方；可信度由 `quality_status` 表示。
pub fn solve_with_evaluators(request: &BlendRequest, evaluators: &EvaluatorSet) -> BlendResult {
    solve_internal(request, evaluators, true)
}

/// `diagnose = false` 的那一路是给 [`relax_target`] 的试解用的: 它要拿真流程验证
/// "合同改成这个数到底解不解得出来", 但自己不能再触发诊断 —— 否则诊断里试解、
/// 试解里又诊断, 一层套一层.
fn solve_internal(
    request: &BlendRequest,
    evaluators: &EvaluatorSet,
    diagnose: bool,
) -> BlendResult {
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

    // 只有 Hard/Priced 约束会剔除缺字段煤；Soft/Advisory 不能改变候选煤池。
    // Priced 同样入列: 它的拒收线是 LP 硬行, 缺输入时无法建约束, 必须和 Hard 一样剔煤。
    let mut required: HashSet<String> = active_specs
        .iter()
        .filter(|spec| matches!(spec.enforcement, Enforcement::Hard | Enforcement::Priced))
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
            // 自身化验值越过采购合同拒收线的煤根本收不进来, 与缺指标同样剔出煤池.
            match cif_eff(coal) {
                Err(reason) => warnings.push(format!(
                    "剔除 {}: {} 实测 {} 越过采购合同拒收线 {}",
                    coal.name,
                    label_zh(&reason.indicator),
                    reason.value,
                    reason.reject
                )),
                Ok(_) => {
                    // 缺化验值的采购条款只是算不出扣款, 不该废掉这个煤 —— 全局模板下
                    // 化验单缺项是常态. 留煤, 但要让用户知道这几项没计进扣款.
                    let skipped = clauses_missing_assay(coal);
                    if !skipped.is_empty() {
                        warnings.push(format!(
                            "{}: 采购条款缺化验值 {}, 本次不计买入扣款",
                            coal.name,
                            skipped
                                .iter()
                                .map(|indicator| label_zh(indicator))
                                .collect::<Vec<_>>()
                                .join("/")
                        ));
                    }
                    kept.push(coal);
                }
            }
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
            matches!(spec.enforcement, Enforcement::Hard | Enforcement::Priced)
                && !formulas.contains_key(&spec.indicator)
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
                    // 这里不带诊断: fallback 有值就说明按合同界建的 LP 本来有解 (previous
                    // 就是那个解), 无解的是岩相修复把上限收紧之后的 LP. 诊断的前提是
                    // "按合同界建的 LP 无解", 在这条路上不成立 —— 硬跑会拿收紧后的代理界
                    // 判越界、却按合同界展示, 报出"岩相 ≤0.1 达不到, 最好只能到 0.05"
                    // 这种自相矛盾的行. 真凶已由 reason 与 warnings 指名为岩相.
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
                // 只有这条路满足诊断的前提: fallback 为 None ⇒ 一次成功迭代都没有过
                // ⇒ petro_proxy_* 必然还是 None (它们与 fallback 在同一处赋值),
                // 失败的就是按合同界建的那个 LP 本身.
                debug_assert!(
                    petro_proxy_lower.is_none() && petro_proxy_upper.is_none(),
                    "未经岩相修复就走到这里, 代理界不该已被收紧"
                );
                let bounds = if diagnose {
                    diagnose_infeasible(&kept, &active_specs, request, &formulas, evaluators)
                } else {
                    Vec::new()
                };
                BlendResult::infeasible("约束冲突, LP 不可行", warnings)
                    .with_infeasible_bounds(bounds)
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
        // 以下两处不可行不带 infeasible_bounds: LP 本身有解 (result 就是 LP 的解),
        // 卡住的是 LP 之后的岩相精确复核. 逐项诊断的推理以"LP 无解"为前提, 这里前提
        // 不成立, 跑了也只会全部落空 —— 真凶已由 reason 与 warnings 指名为岩相.
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

/// 一条计价约束在 LP 中占用的档位列区间.
struct PricedBlock {
    indicator: String,
    /// 档位变量的起始列下标.
    offset: usize,
    /// 各档扣款率 (元/吨 per 1 指标单位), 长度即本块列数.
    rates: Vec<f64>,
}

/// 为每条计价约束追加档位列与三类行, 返回各块的列区间.
///
/// 与 [`append_hard_model_domains`] 同一个接缝形状: 就地追加进 `costs`/`inequalities`/`bounds`.
///
/// **调用方必须在本函数之前写完 `costs` 的配比段**——本函数只在尾部追加档位列的
/// rate, 之后再覆写 `costs` 会把扣款率一起冲掉, 且能编译通过、只是算错钱.
fn append_priced_blocks(
    specs: &[&Spec],
    formulas: &HashMap<String, MetricFormula>,
    request: &BlendRequest,
    ratio_count: usize,
    costs: &mut Vec<f64>,
    inequalities: &mut Vec<Vec<f64>>,
    bounds: &mut Vec<f64>,
) -> Option<Vec<PricedBlock>> {
    debug_assert_eq!(
        costs.len(),
        ratio_count,
        "append_priced_blocks 必须在配比段写完后调用"
    );
    let priced: Vec<&Spec> = specs
        .iter()
        .filter(|spec| spec.enforcement == Enforcement::Priced)
        .copied()
        .collect();

    // 先分配列: 三类行都要按最终总列数补宽, 故列数必须先定下来.
    let mut blocks: Vec<PricedBlock> = Vec::new();
    for spec in &priced {
        let penalty = spec.penalty.as_ref()?;
        let rates: Vec<f64> = penalty.tiers.iter().map(|tier| tier.rate).collect();
        blocks.push(PricedBlock {
            indicator: spec.indicator.clone(),
            offset: costs.len(),
            rates: rates.clone(),
        });
        costs.extend(rates);
    }
    let total_columns = costs.len();

    for (spec, block) in priced.iter().zip(&blocks) {
        let formula = formulas.get(&spec.indicator)?;
        let penalty = spec.penalty.as_ref()?;
        let rule = acceptance_rule(spec, request.truncate_decimal);
        let margin = spec.margin.unwrap_or(0.0);

        // 合同界是精确的合同数字, 不加 margin; margin 只收紧拒收线.
        let (limit, reject) = match spec.direction {
            Direction::Upper => (
                effective_upper(spec.max?, &rule),
                effective_upper(penalty.reject, &rule) - margin,
            ),
            Direction::Lower => (
                effective_lower(spec.min?, &rule),
                effective_lower(penalty.reject, &rule) + margin,
            ),
            Direction::Range => return None,
        };
        let row_for = |target: f64| -> Option<(Vec<f64>, f64)> {
            let (mut row, bound) = match spec.direction {
                Direction::Upper => formula.upper_constraint(target),
                Direction::Lower => formula.lower_constraint(target),
                Direction::Range => return None,
            };
            // 补宽到总列数: build_csc 会零填充短行, 少补不会报错, 只会静默失真.
            row.resize(total_columns, 0.0);
            Some((row, bound))
        };

        // 吸收行与悬崖行只差档位列上那一串 -1: 有 -1 才能"花钱买超标",
        // 没有 -1 就是无人承接的硬墙. 这一处差异即"可计价"与"拒收"的全部分界.
        let (mut row, bound) = row_for(limit)?;
        for index in 0..block.rates.len() {
            row[block.offset + index] = -1.0;
        }
        inequalities.push(row);
        bounds.push(bound);

        // 档宽行: 末档无上限, 不生成.
        for (index, tier) in penalty.tiers.iter().enumerate() {
            if let Some(width) = tier.width {
                let mut row = vec![0.0; total_columns];
                row[block.offset + index] = 1.0;
                inequalities.push(row);
                bounds.push(width);
            }
        }

        let (row, bound) = row_for(reject)?;
        inequalities.push(row);
        bounds.push(bound);
    }
    Some(blocks)
}

/// 从解向量里按块读回各指标的扣款额 (元/吨).
fn read_back_penalties(blocks: &[PricedBlock], solution: &[f64]) -> HashMap<String, f64> {
    blocks
        .iter()
        .map(|block| {
            let amount: f64 = block
                .rates
                .iter()
                .enumerate()
                .map(|(index, rate)| rate * solution[block.offset + index].max(0.0))
                .sum();
            (block.indicator.clone(), amount)
        })
        .collect()
}

/// 一条约束在 LP 里真正执行的界, 每项 = (方向, 合同界, LP 界).
///
/// 合同界是合同白纸黑字那个数, 给人看; LP 界还折算了合同判定规则 (截断/四舍五入)
/// 与安全余量 margin —— Upper 减 margin, Lower 加 margin. Range 拆成上下两条,
/// petro 的 LP 界会被启发式代理覆盖.
///
/// Hard 的硬界是合同 min/max; Priced 的合同界并不硬 (超了按档位折成扣款),
/// 真正硬的是拒收线, 故返回拒收线 —— 与 [`append_priced_blocks`] 的悬崖行必须是
/// 同一条线, 改一处必须改另一处.
///
/// 建 Hard 行与不可行诊断共用本函数: 两处各写一遍, 符号或 margin 迟早漂移.
fn enforced_bounds(
    spec: &Spec,
    request: &BlendRequest,
    petro_proxy_lower: Option<f64>,
    petro_proxy_upper: Option<f64>,
) -> Vec<(Direction, f64, f64)> {
    let (upper_source, lower_source) = match spec.enforcement {
        Enforcement::Hard => (
            matches!(spec.direction, Direction::Upper | Direction::Range)
                .then_some(spec.max)
                .flatten(),
            matches!(spec.direction, Direction::Lower | Direction::Range)
                .then_some(spec.min)
                .flatten(),
        ),
        Enforcement::Priced => {
            let reject = spec.penalty.as_ref().map(|penalty| penalty.reject);
            match spec.direction {
                Direction::Upper => (reject, None),
                Direction::Lower => (None, reject),
                // validate_request 已挡掉 Priced + Range.
                Direction::Range => (None, None),
            }
        }
        // 不进 LP, 谈不上"执行的界".
        Enforcement::Soft | Enforcement::Advisory => return Vec::new(),
    };

    let rule = acceptance_rule(spec, request.truncate_decimal);
    let margin = spec.margin.unwrap_or(0.0);
    let mut limits = Vec::new();
    if let Some(maximum) = upper_source {
        let enforced = effective_upper(maximum, &rule) - margin;
        let enforced = if spec.indicator == "petro" {
            petro_proxy_upper.unwrap_or(enforced)
        } else {
            enforced
        };
        limits.push((Direction::Upper, maximum, enforced));
    }
    if let Some(minimum) = lower_source {
        let enforced = effective_lower(minimum, &rule) + margin;
        let enforced = if spec.indicator == "petro" {
            petro_proxy_lower.unwrap_or(enforced)
        } else {
            enforced
        };
        limits.push((Direction::Lower, minimum, enforced));
    }
    limits
}

/// 反解"合同上那个数要改成多少才真能解出配方", 并**逐档真解一遍**确认.
///
/// 两件事都不能省:
///
/// 一, 不能用差值推. `effective_upper`/`effective_lower` 把合同界折成执行界是阶梯
/// 函数, 按 1/10^decimals 跳档, 合同界在同一档内挪动执行界一动不动. 拿"差多少"去
/// 放宽, 两个方向都会落回原档 —— 截断向要够到 11.3125 却只放宽到 11.2126, 四舍五入
/// 向要够到 10.06 却只放宽到 10.06, 执行界都没挪窝.
///
/// 二, 光算对判定规则这一层还不够, 必须真解. 放宽某项之后, 最低成本解会把这项顶到
/// 新界上 (便宜煤总是更差), 于是最优点正好落在界上, 只差浮点收敛那一丝 —— 认不认
/// 由容限说了算. 两边的容限曾经不是一回事: LP 用相对判据, 解后复核用绝对 1e-8,
/// 实测过 A≤11.3 时解落在界外 1.05e-8, LP 认、复核不认, 整单被判不可行, relax_to
/// 只好再让一档给到 11.4. 现在两边共用 `FEASIBILITY_TOLERANCE`, 那一档不再白丢.
///
/// 口径统一了也仍要真解: 放宽这一项**未必**就够 (还有岩相精确复核那一关), 而
/// "够不够"只有解出来才知道.
///
/// 所以这里逐档往外试, 每档都跑一遍真流程 (`diagnose = false`, 防止试解里再诊断),
/// 第一个真解得出配方的档位才是答案.
///
/// 全都试不通就返回 `None` —— **不拿没试过的数充数**. 放宽这一项可能只是必要而不
/// 充分: LP 通了却卡在岩相精确复核, 就是每一档都解不出来. 界面照着这个数承诺
/// "改成它就能解出配方", 给不出就得让界面说给不出.
fn relax_target(
    spec: &Spec,
    request: &BlendRequest,
    models: &EvaluatorSet,
    direction: Direction,
    achievable: f64,
) -> Option<f64> {
    let rule = acceptance_rule(spec, request.truncate_decimal);
    let margin = spec.margin.unwrap_or(0.0);
    let tolerance = rule.tolerance.max(0.0);
    let fits = |bound: f64| match direction {
        Direction::Upper => effective_upper(bound, &rule) - margin >= achievable,
        Direction::Lower => effective_lower(bound, &rule) + margin <= achievable,
        Direction::Range => false,
    };
    let (target, sign) = match direction {
        Direction::Upper => (achievable + margin, 1.0),
        _ => (achievable - margin, -1.0),
    };

    let (start, step) = match rule.decimals.filter(|_| rule.mode != AcceptanceMode::Raw) {
        Some(decimals) => {
            let scale = 10_f64.powi(i32::from(decimals.min(6)));
            let aligned = match direction {
                Direction::Upper => (target * scale).floor() / scale,
                _ => (target * scale).ceil() / scale,
            };
            (aligned, 1.0 / scale)
        }
        // Raw: 执行界 = 合同界 ± tolerance, 连续可逆; 步长只用来兜浮点边界.
        // "非 Raw 却没有 decimals"也落到这里, 但 validate_request 会先把那种规则挡掉;
        // 万一漏进来, 每档都 fits 不过, 结果是 None —— 退化成"说不出该填几", 不会编数.
        None => (
            target - sign * tolerance,
            4.0 * f64::EPSILON * target.abs().max(1.0),
        ),
    };

    for offset in 0..=RELAX_SEARCH_STEPS {
        let candidate = start + sign * step * f64::from(offset);
        // 先过判定规则这一关: 连执行界都容不下 achievable 的档位, 不必浪费一次求解.
        if !fits(candidate) {
            continue;
        }
        if solve_internal(
            &request_with_bound(request, spec, direction, candidate),
            models,
            false,
        )
        .ok
        {
            return Some(candidate);
        }
    }
    None
}

/// 放宽档位最多往外试几档. 判定规则本身最多差两档 (Round 最坏情况
/// `effective_upper(b) >= b − 量子/2 − eps`), 余下的留给"落在界上、解后复核不认"
/// 那种需要再让一档的情况.
const RELAX_SEARCH_STEPS: u8 = 6;

/// 把某条 spec 的界改成 `bound` 之后的请求副本, 供试解用.
///
/// 与展示口径一致: Priced 改的是拒收线 (它才是硬的), 其余改合同上下界.
///
/// 按 indicator 认人: 之所以不会误伤第二条同指标的约束, 是因为 `validate_request`
/// 已拒绝"同一指标出现两条启用的 spec". 那条校验松掉的话, 这里会一次改多条.
fn request_with_bound(
    request: &BlendRequest,
    target: &Spec,
    direction: Direction,
    bound: f64,
) -> BlendRequest {
    let mut relaxed = request.clone();
    for spec in relaxed
        .specs
        .iter_mut()
        .filter(|spec| spec.enabled && spec.indicator == target.indicator)
    {
        if let (Enforcement::Priced, Some(penalty)) = (spec.enforcement, spec.penalty.as_mut()) {
            penalty.reject = bound;
            continue;
        }
        match direction {
            Direction::Upper => spec.max = Some(bound),
            Direction::Lower => spec.min = Some(bound),
            Direction::Range => {}
        }
    }
    relaxed
}

/// 组装 LP: Hard 行 + 已验证模型训练域 + 计价档位块.
///
/// 不可行诊断复用它: `relaxed` 指定那条被松开的 spec 在 `specs` 里的下标, 得到的就是
/// "少了这条界"的同一个问题, 不必另写一套建模代码.
///
/// **`relaxed` 只松开这条 spec 的界行 (Hard 的上下界行 / Priced 的整个档位块),
/// 模型训练域行照旧保留.** 训练域是"已验证模型可以用于 Hard 约束"的前提, 不属于这条
/// 界本身: 一并删掉等于一次放宽了两层, achievable 会偏乐观, "单独放宽这一项就能可行"
/// 随之变成空话. 今天 solve_json 走默认评估器看不出来, CSR 回归一上线就会现形.
#[allow(clippy::too_many_arguments)]
fn build_lp(
    coals: &[&Coal],
    specs: &[&Spec],
    request: &BlendRequest,
    formulas: &HashMap<String, MetricFormula>,
    models: &EvaluatorSet,
    petro_proxy_lower: Option<f64>,
    petro_proxy_upper: Option<f64>,
    relaxed: Option<usize>,
) -> Option<(LpProblem, Vec<PricedBlock>)> {
    let count = coals.len();
    // 买入侧扣款与水分折算已折进成本系数; 越拒收线的煤在候选筛选阶段已剔除, 此处兜底用报价.
    let mut costs: Vec<f64> = coals.iter().map(|coal| cif_eff_or_quoted(coal)).collect();
    let mut inequalities = Vec::new();
    let mut bounds = Vec::new();

    let bounded: Vec<&Spec> = specs
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != relaxed)
        .map(|(_, spec)| *spec)
        .collect();

    for spec in bounded
        .iter()
        .filter(|spec| spec.enforcement == Enforcement::Hard)
    {
        let formula = formulas.get(&spec.indicator)?;
        for (direction, _, limit) in
            enforced_bounds(spec, request, petro_proxy_lower, petro_proxy_upper)
        {
            let (row, bound) = match direction {
                Direction::Upper => formula.upper_constraint(limit),
                Direction::Lower => formula.lower_constraint(limit),
                Direction::Range => {
                    // enforced_bounds 只产出 Upper/Lower 两向, Range 在那里就拆开了.
                    // 不 panic: 两个 crate 的 release profile 都是 panic = "abort",
                    // 求解线程一炸整个服务端进程跟着没, CatchPanicLayer 也接不住.
                    debug_assert!(false, "enforced_bounds 不该产出 Range");
                    return None;
                }
            };
            inequalities.push(row);
            bounds.push(bound);
        }
    }
    // 训练域行用未过滤的 specs —— 见上面函数级注释.
    append_hard_model_domains(coals, specs, models, &mut inequalities, &mut bounds)?;

    let blocks = append_priced_blocks(
        &bounded,
        formulas,
        request,
        count,
        &mut costs,
        &mut inequalities,
        &mut bounds,
    )?;
    let total_columns = costs.len();

    Some((
        LpProblem {
            ratio_count: count,
            n: total_columns,
            c: costs,
            a_ub: inequalities,
            b_ub: bounds,
        },
        blocks,
    ))
}

/// LP 不可行时逐项定位真凶.
///
/// 做法: 对每条进入 LP 的硬性约束, 重建一个"只删掉它自己那几行"的子问题, 目标换成
/// 该指标本身 —— Upper 求最小, Lower 求最大.
///
/// 逻辑: 整体不可行而子问题可行时, 子问题的最优值**必然**越过被删掉的那条界. 否则
/// 那个解同时满足"其余全部约束"和"这条界", 整体问题就可行了 —— 与前提矛盾. 所以
/// 每条子问题可解的约束都是带确定数字的真凶: 单独把它放宽到最优值就能求出配方.
///
/// 但 `solve_once` 返回 None 不止"LP 无解"一种原因 (还有解后 Hard 复核判 Fail),
/// 那时上面的前提不成立, 推理会凭空指认无辜约束. 所以最后仍显式复核"最优值确实越过
/// LP 界"才计入 —— LP 可行时这一复核必然全部落空, 诊断自动退化成空表.
///
/// 全部子问题都不可行 ⇒ 放宽任何单独一项都不够, 冲突至少牵涉两条约束 (或煤池本身).
/// 此时返回空表, 由调用方如实告诉用户, 而不是硬凑一个真凶.
fn diagnose_infeasible(
    coals: &[&Coal],
    specs: &[&Spec],
    request: &BlendRequest,
    formulas: &HashMap<String, MetricFormula>,
    models: &EvaluatorSet,
) -> Vec<InfeasibleBound> {
    // 岩相启发式代理只在修复迭代里存在, 而那条路不跑诊断 (见调用处), 故这里恒为 None:
    // 诊断面对的永远是"按合同界建的那个 LP".
    let (petro_proxy_lower, petro_proxy_upper) = (None, None);
    let mut culprits = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        let limits = enforced_bounds(spec, request, petro_proxy_lower, petro_proxy_upper);
        if limits.is_empty() {
            continue;
        }
        let Some(formula) = formulas.get(&spec.indicator) else {
            continue;
        };
        let Some((problem, _)) = build_lp(
            coals,
            specs,
            request,
            formulas,
            models,
            petro_proxy_lower,
            petro_proxy_upper,
            Some(index),
        ) else {
            continue;
        };
        for (direction, required, enforced) in limits {
            let Some(achievable) = problem.optimize_indicator(formula, direction) else {
                continue;
            };
            // 判据用 LP 实际执行的界: 越界与否是 LP 说了算.
            // 两条界都要带出去 —— 合同界是用户在合同上认得的那个数, 执行界才是真正
            // 卡住这一单的线. 有 margin 时两者能差出一整个安全余量, 只报合同界的话,
            // 界面会出现"要求 ≤9, 最好能做到 9"这种看着已经达标、却被说成是元凶的行.
            let slack = FEASIBILITY_TOLERANCE * (1.0 + enforced.abs());
            let violated = match direction {
                Direction::Upper => achievable > enforced + slack,
                Direction::Lower => achievable + slack < enforced,
                Direction::Range => false,
            };
            if violated {
                culprits.push(InfeasibleBound {
                    indicator: spec.indicator.clone(),
                    label_zh: label_zh(&spec.indicator).into(),
                    direction,
                    required,
                    enforced,
                    margin: spec.margin.unwrap_or(0.0),
                    achievable,
                    relax_to: relax_target(spec, request, models, direction, achievable),
                });
            }
        }
    }
    culprits
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
    let (problem, blocks) = build_lp(
        coals,
        specs,
        request,
        formulas,
        models,
        petro_proxy_lower,
        petro_proxy_upper,
        None,
    )?;
    let (solution, _) = problem.solve()?;
    let ratios = solution[..count].to_vec();

    let penalty_by_indicator = read_back_penalties(&blocks, &solution);
    let penalty_per_ton: f64 = penalty_by_indicator.values().sum();

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
    // 逐煤先算"修正价 − 报价"再加权, 而不是拿两个千元级总额相减.
    // 无采购条款时每一项恰是 0.0·x, 求和仍是精确 0; 相减形式数学上等价, 浮点上会留下
    // 1e-13 量级残差, 再乘以总吨数放大成一笔并不存在的买入修正.
    let purchase_adjust_per_ton: f64 = coals
        .iter()
        .zip(&ratios)
        .map(|(coal, ratio)| (cif_eff_or_quoted(coal) - coal.cif()) * ratio)
        .sum();
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
        purchase_adjust_per_ton,
        penalty_per_ton,
        net_per_ton: cif_per_ton + purchase_adjust_per_ton + penalty_per_ton,
        total_purchase_adjust: request
            .total_quantity
            .map(|quantity| quantity * purchase_adjust_per_ton),
        total_penalty: request
            .total_quantity
            .map(|quantity| quantity * penalty_per_ton),
        total_net: request
            .total_quantity
            .map(|quantity| quantity * (cif_per_ton + purchase_adjust_per_ton + penalty_per_ton)),
    };
    let mut orders: Vec<OrderItem> = coals
        .iter()
        .zip(&ratios)
        .filter(|(_, ratio)| **ratio > OUTPUT_RATIO_TOLERANCE)
        .map(|(coal, ratio)| {
            let tons = request.total_quantity.map(|quantity| quantity * ratio);
            let effective = cif_eff_or_quoted(coal);
            OrderItem {
                coal: coal.name.clone(),
                ratio: *ratio,
                tons,
                fob_amount: tons.map(|value| value * coal.fob),
                frt_amount: tons.map(|value| value * coal.frt),
                cif_amount: tons.map(|value| value * coal.cif()),
                cif_eff_per_ton: effective,
                cif_eff_amount: tons.map(|value| value * effective),
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
        // 计价指标超合同界是预期行为(已折算成扣款), 只要未越拒收线就不判 Fail.
        // 拒收线本身是 LP 硬约束, 但非线性评估器的复算值可能与 LP 代理不同, 故仍显式复核.
        //
        // 容限必须与 LP 拒收行同量级: LP 放行 FEASIBILITY_TOLERANCE*(1+magnitude),
        // 这里若仍用绝对 SOLUTION_TOLERANCE, 就会把 LP 认可的解判成 Fail,
        // 经 finalize_quality_status 变成 NeedsReview —— 正确配方被盖上"需要复核".
        //
        // 注意参照值不同, 这是刻意的: LP 悬崖行用 effective_upper(reject)∓margin
        // (含安全余量的代理), 这里比的是原始 penalty.reject (合同白纸黑字那条线).
        // 体检回答的是"是否越过合同拒收线", 不是"是否越过内部安全代理"; 不要把两者改成一致.
        if let Some(spec) = spec {
            if spec.enforcement == Enforcement::Priced && outcome.status == EvaluationStatus::Fail {
                let within_reject = spec.penalty.as_ref().is_some_and(|penalty| {
                    let slack = FEASIBILITY_TOLERANCE * (1.0 + penalty.reject.abs());
                    match spec.direction {
                        Direction::Upper => evaluated <= penalty.reject + slack,
                        Direction::Lower => evaluated + slack >= penalty.reject,
                        Direction::Range => false,
                    }
                });
                if within_reject {
                    outcome.status = EvaluationStatus::TolerancePass;
                }
            }
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
            penalty_per_ton: penalty_by_indicator.get(indicator).copied(),
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
        infeasible_bounds: Vec::new(),
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
    /// 参与 Σx=1 的前若干列 (配比变量).
    ratio_count: usize,
    /// 总列数 = ratio_count + 档位变量数.
    n: usize,
    c: Vec<f64>,
    a_ub: Vec<Vec<f64>>,
    b_ub: Vec<f64>,
}

impl LpProblem {
    fn solve(&self) -> Option<(Vec<f64>, f64)> {
        self.solve_with(&self.c)
    }

    /// 把某个指标推到极值 (Upper 求最小, Lower 求最大), 返回该指标的评估值.
    ///
    /// 只换目标向量, 约束原样复用. 目标只写配比列: 分母恒为 1 且 Σx=1
    /// (由 `test_all_formulas_have_unit_denominators` 钉住), 故最小化 Σ 分子·x
    /// 就是最小化该指标本身; 计价档位列留 0, 它们只影响钱, 不影响指标值.
    fn optimize_indicator(&self, formula: &MetricFormula, direction: Direction) -> Option<f64> {
        let sign = match direction {
            Direction::Upper => 1.0,
            Direction::Lower => -1.0,
            Direction::Range => return None,
        };
        let mut objective = vec![0.0; self.n];
        for (column, coefficient) in formula.numerators.iter().take(self.ratio_count).enumerate() {
            objective[column] = sign * coefficient;
        }
        let (solution, _) = self.solve_with(&objective)?;
        formula.evaluate(&solution[..self.ratio_count])
    }

    /// 同一组约束换一个目标向量求解. 行不复制, 诊断的每次探测都走这里.
    fn solve_with(&self, c: &[f64]) -> Option<(Vec<f64>, f64)> {
        debug_assert!(
            self.ratio_count <= self.n
                && c.len() == self.n
                && self.a_ub.len() == self.b_ub.len()
                && self.a_ub.iter().all(|row| row.len() <= self.n),
            "LpProblem 宽度不一致: ratio_count={}, n={}, c={}",
            self.ratio_count,
            self.n,
            c.len()
        );
        let n = self.n;
        let inequality_count = self.a_ub.len();
        let total_rows = 1 + inequality_count + n;
        let mut triplets = Vec::new();

        for column in 0..self.ratio_count {
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
            c,
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
        let raw_sum: f64 = solution[..self.ratio_count].iter().sum();
        // 非负性同样受档位列不归一化之累: 最优解 d=0 时内点法从下方逼近, 会落在
        // -1e-8 量级. 下面立刻 max(0.0) 夹回, 故放宽到 FEASIBILITY_TOLERANCE 是安全的;
        // 它仍比 OUTPUT_RATIO_TOLERANCE(1e-5) 严两个数量级. raw_sum 保持 SOLUTION_TOLERANCE.
        let raw_valid = solution
            .iter()
            .all(|value| value.is_finite() && *value >= -FEASIBILITY_TOLERANCE)
            && (raw_sum - 1.0).abs() <= SOLUTION_TOLERANCE;
        if !raw_valid {
            return None;
        }
        for value in &mut solution {
            *value = value.max(0.0);
        }
        let normalized_sum: f64 = solution[..self.ratio_count].iter().sum();
        if !normalized_sum.is_finite() || normalized_sum <= 0.0 {
            return None;
        }
        for value in &mut solution[..self.ratio_count] {
            *value /= normalized_sum;
        }
        let valid = self.a_ub.iter().zip(&self.b_ub).all(|(row, bound)| {
            let activity: f64 = row
                .iter()
                .zip(&solution)
                .map(|(coefficient, value)| coefficient * value)
                .sum();
            // 相对判据: 配比列经归一化重投影后残差塌到 ~1e-13, 但档位列不归一化,
            // 残差保持在 Clarabel 的原始收敛量级; 而吸收行在每个计价最优解处按构造都是紧的.
            // 绝对容限会因此否掉正确解 —— 容限必须随行与解的量级缩放.
            let magnitude: f64 = row
                .iter()
                .zip(&solution)
                .map(|(coefficient, value)| (coefficient * value).abs())
                .sum::<f64>()
                .max(bound.abs());
            activity <= bound + FEASIBILITY_TOLERANCE * (1.0 + magnitude)
        });
        let objective = c
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 计价约束行的线性性依赖 den·x = Σx = 1.
    /// 若将来引入非单位分母的 MetricFormula, 罚项行会静默失真 —— 用这条测试钉住.
    #[test]
    fn test_all_formulas_have_unit_denominators() {
        let coals = [
            crate::coal_from_tuple(
                "甲",
                (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            ),
            crate::coal_from_tuple(
                "乙",
                (0.8, 8.0, 26.0, 90.0, 18.0, 0.10, 66.0, 8.0, 1100.0, 0.0),
            ),
        ];
        let refs: Vec<&Coal> = coals.iter().collect();

        let check = |label: &str, formulas: &HashMap<String, MetricFormula>| {
            assert!(!formulas.is_empty(), "{label}: 应至少构造出一个公式");
            for (indicator, formula) in formulas {
                assert_eq!(
                    formula.denominators.len(),
                    refs.len(),
                    "{label}: {indicator} 的分母长度应与煤数一致"
                );
                assert!(
                    formula
                        .denominators
                        .iter()
                        .all(|value| (value - 1.0).abs() < 1e-12),
                    "{label}: {indicator} 的分母非单位, 计价约束将失真"
                );
            }
        };

        // 无评估器: 只走 formula_for 的 MetricFormula::linear 路径.
        check(
            "默认评估器",
            &build_formulas(&refs, &EvaluatorSet::default()),
        );

        // 有评估器: 覆盖 build_formulas 里两处手写 MetricFormula —— G 仿射标定与
        // CSR 回归. 这两处是最可能被引入非单位分母的地方, 默认评估器根本到不了.
        let trained = trained_evaluators();
        let formulas = build_formulas(&refs, &trained);
        assert_eq!(
            formulas["G"].method,
            EvaluationMethod::AffineCalibration,
            "应走到 G 仿射标定分支"
        );
        assert_eq!(
            formulas["CSR"].method,
            EvaluationMethod::Regression,
            "应走到 CSR 回归分支"
        );
        check("已训练评估器", &formulas);
    }

    /// 直接构造两个已门控模型, 用于把 `build_formulas` 的手写 MetricFormula 分支走到.
    /// 不用 `EvaluatorSet::train`: 那会把测试耦合到样本量门槛等训练策略上.
    fn trained_evaluators() -> EvaluatorSet {
        EvaluatorSet {
            g: Some(crate::predict::ValidatedGModel {
                predictor: crate::predict::GAffinePredictor {
                    intercept: 5.0,
                    slope: 0.9,
                    sample_count: 30,
                },
                version: "test-g".into(),
                cv_mae: 1.0,
                p90_abs_error: 2.0,
                bias: 0.0,
                training_min: 60.0,
                training_max: 100.0,
            }),
            csr: Some(crate::predict::ValidatedCsrModel {
                predictor: crate::predict::CsrPredictor {
                    intercept: 20.0,
                    beta_s: -1.0,
                    beta_a: -0.5,
                    beta_v: -0.2,
                    beta_g: 0.4,
                    beta_y: 0.3,
                    beta_m: -0.1,
                    r_squared: 0.9,
                    n_samples: 30,
                },
                version: "test-csr".into(),
                cv_mae: 1.0,
                p90_abs_error: 3.0,
                bias: 0.0,
                training_min: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                training_max: [10.0, 20.0, 40.0, 110.0, 30.0, 20.0],
            }),
            warnings: Vec::new(),
            extrapolation_ratio: 0.1,
        }
    }

    /// 档位列不得参与 Σx=1.
    ///
    /// 本测试中附加列最优解为 0, 归一化是否误缩放它不可观测——
    /// 归一化只对配比列生效由 [`test_lp_problem_handles_nonzero_extra_column`] 覆盖.
    #[test]
    fn test_lp_problem_excludes_extra_columns_from_sum() {
        let problem = LpProblem {
            ratio_count: 2,
            n: 3,
            c: vec![10.0, 20.0, 5.0],
            a_ub: Vec::new(),
            b_ub: Vec::new(),
        };
        let (solution, objective) = problem.solve().expect("应可解");
        assert_eq!(solution.len(), 3, "解应含全部列");
        assert!(
            (solution[0] + solution[1] - 1.0).abs() < 1e-6,
            "前 ratio_count 列之和应为 1, 实得 {}",
            solution[0] + solution[1]
        );
        assert!(solution[0] > 0.999, "应全选更便宜的第 0 列");
        assert!(solution[2].abs() < 1e-6, "附加列成本为正, 最优应取 0");
        assert!(
            (objective - 10.0).abs() < 1e-4,
            "目标值应为 10, 实得 {objective}"
        );
    }

    /// 附加列在最优解处非零时, 三处改动(等式行/raw_sum/归一化)才全部可检验——
    /// 本测试是"归一化只对配比列生效"这一断言唯一能证伪的用例.
    ///
    /// 约束 x0 + x1 − d ≤ 0.5 (d 即附加列 solution[2]), 因 Σx = 1 故等价于强制 d ≥ 0.5.
    /// 若 raw_sum 仍对全部列求和: 1 + 0.5 = 1.5 ≠ 1 ⇒ 可行性校验失败 ⇒ solve 返回 None ⇒ 本测试 panic.
    /// 若归一化仍对全部列做: 配比被 1.5 除 ⇒ 前两列之和变成 0.667 ⇒ 断言失败.
    #[test]
    fn test_lp_problem_handles_nonzero_extra_column() {
        let problem = LpProblem {
            ratio_count: 2,
            n: 3,
            c: vec![10.0, 20.0, 5.0],
            a_ub: vec![vec![1.0, 1.0, -1.0]],
            b_ub: vec![0.5],
        };
        let (solution, objective) = problem.solve().expect("应可解");
        assert!(
            (solution[0] + solution[1] - 1.0).abs() < 1e-6,
            "配比列之和应为 1, 实得 {}",
            solution[0] + solution[1]
        );
        assert!(
            (solution[2] - 0.5).abs() < 1e-5,
            "附加列应被约束逼到 0.5, 实得 {}",
            solution[2]
        );
        // 10·1 + 20·0 + 5·0.5 = 12.5
        assert!(
            (objective - 12.5).abs() < 1e-4,
            "目标值应为 12.5, 实得 {objective}"
        );
    }

    // ========================================================================
    // 不可行诊断
    // ========================================================================

    /// 只带化验六项的煤 (无 petro/CSR), 与用户实际录入的化验单一致.
    fn assay_coal(name: &str, assay: (f64, f64, f64, f64, f64, f64), fob: f64) -> Coal {
        let (s, a, v, g, y, m) = assay;
        Coal {
            name: name.into(),
            props: [("S", s), ("A", a), ("V", v), ("G", g), ("Y", y), ("M", m)]
                .into_iter()
                .map(|(indicator, value)| (indicator.to_string(), value))
                .collect(),
            fob,
            frt: 0.0,
            petrography: None,
            purchase_terms: None,
        }
    }

    /// 用户线上撞到不可行的那个煤池.
    pub(super) fn real_case_coals() -> Vec<Coal> {
        vec![
            assay_coal("兴无", (2.1, 8.5, 18.0, 70.0, 18.0, 12.0), 2400.0),
            assay_coal("第三", (0.73, 12.14, 34.0, 85.0, 15.0, 12.0), 1550.0),
            assay_coal("第二", (0.78, 11.5, 23.0, 86.0, 15.0, 12.0), 2125.0),
            assay_coal("第一", (1.0, 13.85, 21.0, 85.0, 15.0, 12.0), 2465.0),
        ]
    }

    fn request_of(coals: Vec<Coal>, specs: Vec<Spec>) -> BlendRequest {
        BlendRequest {
            coals,
            specs,
            total_quantity: Some(3_700.0),
            truncate_decimal: true,
        }
    }

    /// 用户线上那一单: 灰 ≤10 在这个煤池里根本做不到, 而报错只说"约束冲突".
    ///
    /// 五条约束同时在场是这条测试的关键 —— 只有一条时"指认真凶"和"把在场的都报一遍"
    /// 看不出区别.
    #[test]
    fn test_infeasible_diagnosis_names_ash_with_a_number() {
        let result = solve(&request_of(
            real_case_coals(),
            vec![
                Spec::upper("A", 10.0),
                Spec::upper("S", 1.0),
                Spec::upper("V", 28.0),
                Spec::lower("Y", 15.0),
                Spec::lower("G", 85.0),
            ],
        ));

        assert!(!result.ok, "这份合同在这个煤池里应当不可行");
        let named: Vec<&str> = result
            .infeasible_bounds
            .iter()
            .map(|bound| bound.indicator.as_str())
            .collect();
        assert_eq!(
            named,
            ["A"],
            "只有单独放宽灰分才能救活这一单, 其余四项不该被指认"
        );

        let ash = &result.infeasible_bounds[0];
        assert_eq!(ash.label_zh, "灰");
        assert_eq!(ash.direction, Direction::Upper);
        assert_eq!(ash.required, 10.0, "要展示合同界, 不是 LP 折算后的界");
        // 手算: G≥85 把兴无 (G=70) 压到 1/16 以内, 而兴无是唯一灰分低于 10 的煤,
        // 余量只能给灰分最低的第二 (11.5): 8.5×1/16 + 11.5×15/16 = 11.3125.
        assert!(
            (ash.achievable - 11.3125).abs() < 0.01,
            "灰分最低只能到 11.31, 实得 {}",
            ash.achievable
        );
    }

    /// 计价指标的硬界是拒收线, 不是合同界 —— 超合同界只是扣款, 越拒收线才不可行.
    /// 这里合同上限 9 与拒收线 10 刻意分开: 拿错哪个当"合同要求"都会被这条测出来.
    #[test]
    fn test_priced_diagnosis_reports_the_reject_line() {
        let mut ash = Spec::upper("A", 9.0);
        ash.enforcement = Enforcement::Priced;
        ash.penalty = Some(Penalty {
            tiers: vec![
                PenaltyTier {
                    width: Some(0.5),
                    rate: 10.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 50.0,
                },
            ],
            reject: 10.0,
        });
        // 关掉截断判定, 让手算的界就是 LP 的界.
        let result = solve(&BlendRequest {
            coals: real_case_coals(),
            specs: vec![ash, Spec::lower("G", 85.0)],
            total_quantity: Some(3_700.0),
            truncate_decimal: false,
        });

        assert!(!result.ok, "灰分连拒收线 10 都够不到, 应当不可行");
        let ash_bound = result
            .infeasible_bounds
            .iter()
            .find(|bound| bound.indicator == "A")
            .expect("应指认灰分");
        assert_eq!(
            ash_bound.required, 10.0,
            "计价项要展示拒收线 10, 不是合同上限 9"
        );
        assert!(
            (ash_bound.achievable - 11.3125).abs() < 0.01,
            "灰分最低仍是 11.31, 实得 {}",
            ash_bound.achievable
        );

        // 粘结这一侧同样是真凶: 少了硫/挥发/胶质的牵制, 单独把 G 放到 78 也能可行.
        // 手算: 灰 ≤10 要兴无占一半 (11.5−3x≤10 ⇒ x≥0.5), 粘结随之被压到 86−16×0.5=78.
        let g_bound = result
            .infeasible_bounds
            .iter()
            .find(|bound| bound.indicator == "G")
            .expect("粘结单独放宽也能救活, 应一并指认");
        assert_eq!(g_bound.direction, Direction::Lower);
        assert_eq!(g_bound.required, 85.0);
        assert!(
            (g_bound.achievable - 78.0).abs() < 0.01,
            "粘结最高只能到 78, 实得 {}",
            g_bound.achievable
        );
        assert_eq!(result.infeasible_bounds.len(), 2, "只有这两项能单独放宽");
    }

    /// 区间约束上下两侧都可能是真凶, 诊断必须说清是哪一侧.
    /// 这里挥发下限 10 轻松满足, 够不到的只有上限 15.
    #[test]
    fn test_range_diagnosis_picks_the_violated_side() {
        let result = solve(&request_of(
            real_case_coals(),
            vec![Spec::range("V", 10.0, 15.0)],
        ));

        assert!(!result.ok, "全煤池挥发都在 18 以上, 上限 15 做不到");
        assert_eq!(
            result.infeasible_bounds.len(),
            1,
            "只有上限够不到, 下限一侧不该也报一条: {:?}",
            result.infeasible_bounds
        );
        let bound = &result.infeasible_bounds[0];
        assert_eq!(bound.indicator, "V");
        assert_eq!(bound.direction, Direction::Upper);
        assert_eq!(bound.required, 15.0);
        // 兴无挥发 18 最低, 其余都更高.
        assert!(
            (bound.achievable - 18.0).abs() < 0.01,
            "挥发最低只能到 18, 实得 {}",
            bound.achievable
        );
    }

    /// 三条约束两两都冲突时, 放宽任何单独一项都救不回来 —— 此时要如实交白卷,
    /// 而不是随便指一个煤池里最紧的指标当真凶.
    #[test]
    fn test_diagnosis_stays_empty_when_no_single_spec_explains() {
        // 每种煤只在一项上过关, 另两项都远超界: 任意两条约束同时在场就已经不可行.
        let coals = vec![
            assay_coal("硫优", (1.0, 20.0, 40.0, 85.0, 15.0, 12.0), 1000.0),
            assay_coal("灰优", (10.0, 2.0, 40.0, 85.0, 15.0, 12.0), 1000.0),
            assay_coal("挥优", (10.0, 20.0, 4.0, 85.0, 15.0, 12.0), 1000.0),
        ];
        let result = solve(&request_of(
            coals,
            vec![
                Spec::upper("S", 2.0),
                Spec::upper("A", 4.0),
                Spec::upper("V", 8.0),
            ],
        ));

        assert!(!result.ok, "三项两两冲突, 应当不可行");
        assert_eq!(result.reason.as_deref(), Some("约束冲突, LP 不可行"));
        assert!(
            result.infeasible_bounds.is_empty(),
            "没有哪一项单独放宽能可行, 不该指认任何一项: {:?}",
            result.infeasible_bounds
        );
    }

    /// 子问题只松开被诊断那一条界, 模型训练域行必须留着.
    ///
    /// G 已验证模型: 评估值 = 5 + 0.9·原始G, 训练域 [60,100] 外扩 10% 后是 [56,104].
    /// 合同要粘结 ≥100 ⇒ 原始G 需 ≥105.6, 越过训练域上限 104 ⇒ 不可行.
    /// 诊断删掉粘结那条界后, 原始G 仍被训练域封在 104, 故最多做到 5+0.9×104 = 98.6.
    ///
    /// 若把训练域行跟着一起删掉 (append_hard_model_domains 是按 spec 挂的, 很容易
    /// 顺手删掉), 原始G 就能顶到煤本身的 110 ⇒ 评估值 104 ≥ 100 ⇒ 复核认为没越界 ⇒
    /// 这一项根本不会被报出来, 断言随之失败. 今天 solve_json 走默认评估器看不到这条路,
    /// CSR/G 回归一上线就是生产路径.
    #[test]
    fn test_diagnosis_keeps_model_domain_rows() {
        let request = request_of(
            vec![
                assay_coal("高粘结", (0.8, 9.0, 24.0, 110.0, 16.0, 10.0), 1200.0),
                assay_coal("普通", (0.8, 9.0, 24.0, 90.0, 16.0, 10.0), 1000.0),
            ],
            vec![Spec::lower("G", 100.0)],
        );
        let request = BlendRequest {
            truncate_decimal: false,
            ..request
        };
        let result = solve_with_evaluators(&request, &trained_evaluators());

        assert!(!result.ok, "原始G 被训练域封在 104, 评估值到不了 100");
        assert_eq!(result.infeasible_bounds.len(), 1, "应指认粘结一项");
        let bound = &result.infeasible_bounds[0];
        assert_eq!(bound.indicator, "G");
        assert_eq!(bound.direction, Direction::Lower);
        assert_eq!(bound.required, 100.0);
        assert!(
            (bound.achievable - 98.6).abs() < 0.01,
            "训练域上限 104 ⇒ 最多 98.6, 实得 {} (104 说明训练域行被一起删了)",
            bound.achievable
        );
    }

    /// 诊断前提的兜底: LP 可行、卡在解后 Hard 复核时, 一项都不许指认.
    ///
    /// 这里 G 模型输出 50+0.9×90 = 131, 超出 0~100 物理范围被判 Fail, solve_once
    /// 因此返回 None —— 但 LP 本身有解. "整体不可行 ⇒ 子问题最优值必然越界"的推理
    /// 在这种 None 上不成立, 全靠最后那道"确实越界"的复核兜住: 粘结最多能到 131,
    /// 远在下限 85 之上, 复核落空, 于是交白卷.
    #[test]
    fn test_feasible_lp_failing_post_check_names_nobody() {
        let mut models = trained_evaluators();
        models
            .g
            .as_mut()
            .expect("测试评估器应带 G 模型")
            .predictor
            .intercept = 50.0;
        let request = request_of(
            vec![
                assay_coal("甲", (0.8, 9.0, 24.0, 90.0, 16.0, 10.0), 1000.0),
                assay_coal("乙", (0.9, 9.5, 25.0, 88.0, 16.0, 10.0), 1100.0),
            ],
            vec![Spec::lower("G", 85.0)],
        );
        let result = solve_with_evaluators(&request, &models);

        assert!(!result.ok, "模型输出越出物理范围, 不该给配方");
        assert!(
            result.infeasible_bounds.is_empty(),
            "LP 可行, 没有哪条约束够不到, 不许指认: {:?}",
            result.infeasible_bounds
        );
    }

    /// 有安全余量时, 合同界与真正卡住这一单的执行界不是同一条线, 两条都要报.
    ///
    /// 合同 灰 ≤9、余量 1.0 ⇒ LP 按 ≤8 执行; 煤池最低只能做到 9.0.
    /// 只报合同界的话界面会是"要求 ≤9, 最好能做到 9" —— 看着已经达标却被指认为元凶,
    /// 而且"把合同放宽到 9 就能可行"是假的: 真正要让开的是 8 那条线.
    #[test]
    fn test_margin_reports_both_contract_and_enforced_bound() {
        let mut ash = Spec::upper("A", 9.0);
        ash.margin = Some(1.0);
        let request = BlendRequest {
            coals: vec![
                assay_coal("低灰", (0.8, 9.0, 24.0, 88.0, 16.0, 10.0), 1000.0),
                assay_coal("高灰", (0.8, 12.0, 24.0, 88.0, 16.0, 10.0), 900.0),
            ],
            specs: vec![ash],
            total_quantity: None,
            truncate_decimal: false,
        };
        let result = solve(&request);

        assert!(!result.ok, "煤池最低 9.0, 够不到收紧后的 8.0");
        assert_eq!(result.infeasible_bounds.len(), 1);
        let bound = &result.infeasible_bounds[0];
        assert_eq!(bound.required, 9.0, "合同界是用户在合同上认得的那个数");
        assert_eq!(bound.enforced, 8.0, "执行界 = 合同界 9 扣掉安全余量 1");
        assert!(
            (bound.achievable - 9.0).abs() < 0.01,
            "最低只能到 9.0, 实得 {}",
            bound.achievable
        );
    }

    /// 按诊断给的"放宽到"改合同, 再解一次.
    ///
    /// relax_to 宣称的是"合同上这个数改成它就能求出配方" —— 唯一说得过去的验收就是
    /// 真改真解. 前两版分别用"差 achievable−合同界"和"差 achievable−执行界"推,
    /// 都没有真解一遍, 于是两种推法各自的反例都溜了过去.
    /// 复用生产代码的 request_with_bound: 测试改合同的方式必须和实现改的是同一处,
    /// 否则验的就不是同一件事.
    fn resolve_after_relaxing(
        request: &BlendRequest,
        bound: &InfeasibleBound,
        relax_to: f64,
    ) -> BlendResult {
        let spec = request
            .specs
            .iter()
            .find(|spec| spec.indicator == bound.indicator)
            .expect("诊断指认的指标应能在请求里找到")
            .clone();
        solve(&request_with_bound(
            request,
            &spec,
            bound.direction,
            relax_to,
        ))
    }

    /// 诊断给了可填的数时取出来; 给不出 (None) 的用例不该走到这里.
    fn expect_relax_to(bound: &InfeasibleBound) -> f64 {
        bound.relax_to.expect("这一档应能试出可填的数")
    }

    /// 下限方向要顶界, 得让"高粘结煤贵、低粘结煤便宜": 最低成本解才会把 G 压到界上.
    fn cohesion_pool() -> Vec<Coal> {
        vec![
            assay_coal("高粘结贵", (0.5, 9.0, 24.0, 92.0, 18.0, 9.0), 2300.0),
            assay_coal("低粘结廉", (0.5, 9.0, 24.0, 71.0, 13.0, 9.0), 1400.0),
            assay_coal("中间", (0.5, 9.0, 24.0, 83.0, 15.0, 9.0), 1800.0),
        ]
    }

    /// 回归: Hard 界顶格时, 解后复核的容限必须与 LP 可行性复核同口径.
    ///
    /// Hard 界一旦卡住, LP 最优解**按定义**就落在界上, 只差浮点收敛那一丝. LP 用相对
    /// 判据 `FEASIBILITY_TOLERANCE × (1 + 量级)` 认下它; 解后复核原本用绝对 1e-8,
    /// 于是同一个解 LP 认、体检不认 —— `solve_once` 返回 None, 可行的合同被报成
    /// "约束冲突, LP 不可行". 计价侧在扣款那一版已经对齐过, Hard 侧漏了.
    ///
    /// 三条用例刻意各不相同, 少一条这测试就说不清覆盖到哪:
    /// - 上限 × 截断判定: 走的是 `slack` 那条比较 (判定值 11.3 本身是达标的);
    /// - 上限 × Raw 判定: Raw 下 judged 就是实测值, 卡人的是 `judged_pass` 那条;
    /// - 下限 × Raw 判定: 上面两条都只走上界分支, 下界是另一行代码.
    #[test]
    fn test_hard_check_tolerance_matches_lp_wall() {
        struct Case {
            label: &'static str,
            coals: Vec<Coal>,
            specs: Vec<Spec>,
            truncate: bool,
            indicator: &'static str,
            bound: f64,
            /// 顶的是上界还是下界.
            upper: bool,
        }

        let cases = vec![
            Case {
                label: "上限×截断: 用户线上那一单, 灰分改到 11.3",
                coals: real_case_coals(),
                specs: vec![
                    Spec::upper("A", 11.3),
                    Spec::upper("S", 1.0),
                    Spec::upper("V", 28.0),
                    Spec::lower("Y", 15.0),
                    Spec::lower("G", 85.0),
                ],
                truncate: true,
                indicator: "A",
                bound: 11.3,
                upper: true,
            },
            Case {
                label: "上限×Raw: 灰分 ≤11.46 不折档, 实测值本身就是判定值",
                coals: real_case_coals(),
                specs: vec![Spec::upper("A", 11.46)],
                truncate: false,
                indicator: "A",
                bound: 11.46,
                upper: true,
            },
            Case {
                label: "下限×Raw: 粘结 ≥73 顶在下界上",
                coals: cohesion_pool(),
                specs: vec![Spec::lower("G", 73.0)],
                truncate: false,
                indicator: "G",
                bound: 73.0,
                upper: false,
            },
        ];

        for case in cases {
            let Case {
                label,
                coals,
                specs,
                truncate,
                indicator,
                bound,
                upper,
            } = case;
            let request = BlendRequest {
                coals,
                specs,
                total_quantity: Some(3_700.0),
                truncate_decimal: truncate,
            };
            let result = solve(&request);
            assert!(
                result.ok,
                "{label}: LP 认下的解不该被体检否掉: {:?}",
                result.reason
            );

            let check = result
                .indicator_check
                .iter()
                .find(|check| check.indicator == indicator)
                .unwrap_or_else(|| panic!("{label}: 应有 {indicator} 体检"));
            let slack = check.slack.expect("带界的指标应有余量");

            // 这三条用例的意义全在"解确实落在执行界外侧一丝". 哪天煤池或求解器变了,
            // 解不再顶界, 上面的 ok 断言就成了空转 —— 这里先把它拦住.
            assert!(
                slack < -1e-8,
                "{label}: 本用例须让解落在执行界外、且超出旧的绝对 1e-8 才有意义, 实得余量 {slack:e}"
            );
            assert_ne!(
                check.status,
                EvaluationStatus::Fail,
                "{label}: LP 已认可的解不应被体检判 Fail (实测 {}, 余量 {slack:e})",
                check.value
            );
            assert!(
                check.binding,
                "{label}: 顶在界上的指标必须标成 binding, 界面的谈判方向靠它"
            );

            // 放行的前提是"报到化验刻度上仍然达标" —— 不是"差得不多就算了".
            let judged = check.judged_value.expect("带界的指标应有判定值");
            let reported = (judged * 100.0).round() / 100.0;
            if upper {
                assert!(
                    reported <= bound,
                    "{label}: 报到 0.01 刻度的 {reported} 仍越过合同界 {bound}"
                );
            } else {
                assert!(
                    reported >= bound,
                    "{label}: 报到 0.01 刻度的 {reported} 仍够不到合同界 {bound}"
                );
            }
        }
    }

    /// Hard 界是硬墙: 超界 0.01 (化验一个刻度) 必须判不可行.
    ///
    /// 与上一条成对, 但**把住的不是同一道门**, 这点别记混:
    /// 上一条管"别把 LP 认下的解否掉" (解后复核那道门); 这一条管"真越界的进不来",
    /// 而 Hard 线性指标真正拦人的是 **LP 那一行** —— 它的界就是 `effective_*`,
    /// 与解后复核同一条线, 所以越界的解根本走不到复核跟前. 实测可证: 把
    /// `ACCEPT_TOLERANCE` 放大四个数量级, 45 万条样本里最差残差纹丝不动, 仍是
    /// 1.25e-7 —— 复核对线性 Hard 指标是冗余的安全网, 不是墙.
    ///
    /// 所以这条测试守的是 LP 那道墙, **不守 `ACCEPT_TOLERANCE` 的上界**: 那个常量
    /// 调松了这条也不会红. 常量的上界靠"与 LP 共用同一个定义"钉死, 不靠断言.
    /// (计价侧的同名守卫是 `test_reject_line_still_blocks_one_assay_increment`;
    /// Hard 侧一直空着, 这里补上.)
    ///
    /// 用 Raw 判定: 截断/四舍五入下合同 ≤10 本来就按 ≤10.0999 执行, 超 0.01 是合同
    /// 认可的, 那时判可行并非放水. 硬墙要在"判定规则不折档"这一口径上验.
    #[test]
    fn test_hard_bound_still_blocks_one_assay_increment() {
        let solves = |assay: (f64, f64, f64, f64, f64, f64), spec: Spec| -> bool {
            solve(&BlendRequest {
                coals: vec![assay_coal("独苗", assay, 1000.0)],
                specs: vec![spec],
                total_quantity: None,
                truncate_decimal: false,
            })
            .ok
        };

        // 上界: 灰 ≤10.
        assert!(
            solves((0.5, 10.0, 24.0, 88.0, 16.0, 9.0), Spec::upper("A", 10.0)),
            "正好压合同上限应可行"
        );
        assert!(
            !solves((0.5, 10.01, 24.0, 88.0, 16.0, 9.0), Spec::upper("A", 10.0)),
            "超合同上限 0.01 (化验一个刻度) 必须判不可行"
        );

        // 下界: 粘结 ≥80. 只验上界的话, 下界那行代码的墙没人验.
        assert!(
            solves((0.5, 9.0, 24.0, 80.0, 16.0, 9.0), Spec::lower("G", 80.0)),
            "正好压合同下限应可行"
        );
        assert!(
            !solves((0.5, 9.0, 24.0, 79.99, 16.0, 9.0), Spec::lower("G", 80.0)),
            "差合同下限 0.01 (化验一个刻度) 必须判不可行"
        );
    }

    /// 用户线上那一单: 合同灰分改到多少才真解得出配方.
    ///
    /// 判定规则层面的最小值是 **11.3** —— 一位小数截断下执行界 = (floor(b×10)+1)/10 − eps,
    /// 要容下 11.3125 就需要 floor(b×10) ≥ 113 ⇒ b ≥ 11.3 (11.2 落回 11.29999 那一档).
    ///
    /// 这个数曾经是 **11.4**: 11.3 放宽之后最低成本解把灰分顶到新界上, 落在界外 1.05e-8,
    /// LP 的相对容限认、解后复核的绝对容限不认, 整单仍判不可行, 于是 relax_to 只好再让
    /// 一档. 容限口径对齐 (见 `test_hard_check_tolerance_matches_lp_wall`) 之后 11.3
    /// 真解得出来了, 期望值随之回到 11.3 —— 期望是跟着缺陷修复走的, 不是被改松的.
    ///
    /// 用户问的是"我该往合同里填几", 不是"数学下限是几", 所以这里必须给真解得出的那个数.
    #[test]
    fn test_relax_to_is_the_bound_that_actually_solves() {
        let request = request_of(
            real_case_coals(),
            vec![
                Spec::upper("A", 10.0),
                Spec::upper("S", 1.0),
                Spec::upper("V", 28.0),
                Spec::lower("Y", 15.0),
                Spec::lower("G", 85.0),
            ],
        );
        let result = solve(&request);
        let ash = &result.infeasible_bounds[0];

        let relax_to = expect_relax_to(ash);
        assert!(
            resolve_after_relaxing(&request, ash, relax_to).ok,
            "改成 relax_to={relax_to} 之后必须真的解得出配方"
        );
        assert!(
            (relax_to - 11.3).abs() < 1e-9,
            "应给真解得出的 11.3, 实得 {relax_to}"
        );

        // 判定规则层面的下限就是 11.3, 现在它真解得出来了.
        assert!(
            resolve_after_relaxing(&request, ash, 11.3).ok,
            "11.3 是判定规则算得出的下限, 容限口径对齐后必须真能解"
        );

        // 再紧一档仍然不行 —— 11.2 的执行界落回 11.29999, 容不下 11.3125.
        // 少了这条, 上面那条断言就只说明"11.3 能解", 说不明"11.3 是最紧的那一档".
        assert!(
            !resolve_after_relaxing(&request, ash, 11.2).ok,
            "11.2 的执行界是 11.29999, 够不到 11.3125, 不该能解"
        );

        // 反例钉死: 差值法给出的 11.2126 连执行界那一档都没挪动.
        let gap_based = ash.achievable - (ash.enforced - ash.required);
        assert!(
            !resolve_after_relaxing(&request, ash, gap_based).ok,
            "差值推出来的 {gap_based} 落在同一档内, 本就解不出来"
        );
    }

    /// 四舍五入判定下的反例: 放宽到 achievable 本身不够, 得跳到下一档.
    /// 合同 ≤10 在 Round(1) 下执行界是 10.04999; 煤池最低 10.06, 放宽到 10.06 后
    /// 执行界还是 10.04999 —— 档位没挪. 正解是 10.1.
    #[test]
    fn test_round_acceptance_relax_to_jumps_a_whole_step() {
        let mut ash = Spec::upper("A", 10.0);
        ash.acceptance = Some(AcceptanceRule {
            mode: AcceptanceMode::Round,
            decimals: Some(1),
            tolerance: 0.0,
        });
        let request = BlendRequest {
            coals: vec![
                assay_coal("略高灰", (0.8, 10.06, 24.0, 88.0, 16.0, 10.0), 1000.0),
                assay_coal("高灰", (0.8, 12.0, 24.0, 88.0, 16.0, 10.0), 900.0),
            ],
            specs: vec![ash],
            total_quantity: None,
            truncate_decimal: false,
        };
        let result = solve(&request);

        assert!(!result.ok, "10.06 越过四舍五入执行界 10.04999");
        let bound = &result.infeasible_bounds[0];
        let relax_to = expect_relax_to(bound);
        assert!(
            (relax_to - 10.1).abs() < 1e-9,
            "应跳到下一档 10.1, 实得 {relax_to}"
        );
        assert!(
            resolve_after_relaxing(&request, bound, relax_to).ok,
            "10.1 必须真能解"
        );
        assert!(
            !resolve_after_relaxing(&request, bound, bound.achievable).ok,
            "放宽到 achievable 本身仍在同一档, 解不出来"
        );
    }

    /// 截断判定在**下限**一侧是收紧的, 且与安全余量无关.
    /// 合同 ≥14.95 一位小数截断后按 ≥15.0 执行 (ceil): 没有任何 margin, 执行界却比
    /// 合同界严. 所以"执行界为什么更紧"不能按方向猜, 只能看 margin 这个成因字段.
    #[test]
    fn test_truncate_tightens_lower_bound_without_any_margin() {
        let mut cohesion = Spec::lower("Y", 14.95);
        cohesion.acceptance = Some(AcceptanceRule {
            mode: AcceptanceMode::Truncate,
            decimals: Some(1),
            tolerance: 0.0,
        });
        let request = BlendRequest {
            coals: vec![
                assay_coal("略低胶质", (0.8, 9.0, 24.0, 88.0, 14.98, 10.0), 1000.0),
                assay_coal("更低胶质", (0.8, 9.0, 24.0, 88.0, 14.0, 10.0), 900.0),
            ],
            specs: vec![cohesion],
            total_quantity: None,
            truncate_decimal: false,
        };
        let result = solve(&request);

        assert!(!result.ok, "最高 14.98 够不到截断后的 15.0");
        let bound = &result.infeasible_bounds[0];
        assert_eq!(bound.required, 14.95);
        assert_eq!(bound.enforced, 15.0, "截断把下限抬到了 15.0");
        assert_eq!(bound.margin, 0.0, "这里一点安全余量都没设");
        let relax_to = expect_relax_to(bound);
        assert!(
            (relax_to - 14.9).abs() < 1e-9,
            "要 ceil(b×10) ≤ 149, 即 b ≤ 14.9, 实得 {relax_to}"
        );
        assert!(resolve_after_relaxing(&request, bound, relax_to).ok);
    }

    /// 可行的一单不带诊断: 诊断只跑在不可行这条路上.
    #[test]
    fn test_feasible_solve_carries_no_diagnosis() {
        let result = solve(&request_of(
            real_case_coals(),
            vec![Spec::upper("A", 13.0), Spec::lower("G", 85.0)],
        ));
        assert!(result.ok, "灰 ≤13 / 粘结 ≥85 在这个煤池里可行");
        assert!(result.infeasible_bounds.is_empty());
    }
}

// ============================================================================
// 容限量级测量 (手动触发, 不进 CI)
// ============================================================================

/// `FEASIBILITY_TOLERANCE` 与 `quality::ACCEPT_TOLERANCE` 注释里记的那些数字,
/// 由本模块的 `measure_tolerance_headroom` 产出:
///
/// ```text
/// cargo test --release -- --ignored measure_tolerance_headroom --nocapture
/// ```
///
/// 为什么要能复跑, 而不是注释里记个数就算: 两个常量都是安全关键的, 余量却很薄
/// (`FEASIBILITY_TOLERANCE` 只有 1.02 倍), 煤池变宽、合同变紧、Clarabel 升级,
/// 任何一样都可能把它吃掉. 一段没法复跑的"实测"在被推翻之前与编造无法区分 ——
/// 上一版就把一个 min 累加器的 sentinel 初值 (1.0000) 当成了实测结果报出去。
///
/// 所以这里的累加器一律 `Option` + 样本计数: "零样本"和"恰好测出这个数"在输出上
/// 必须长得不一样, 见 [`Extremum::report`].
#[cfg(test)]
mod measurements {
    use super::*;
    use crate::predict::{CsrObservation, EvaluatorSet};
    use crate::quality::ACCEPT_TOLERANCE;

    /// 极值累加器. **不允许用可能与真实结果混淆的哨兵初始化** —— 这是上一版的教训:
    /// `f64::INFINITY` 还好认, `1.0` 这种"看着像结果"的初值会直接骗过读的人。
    struct Extremum {
        /// 取最大还是最小.
        maximize: bool,
        samples: usize,
        best: Option<f64>,
        witness: String,
    }

    impl Extremum {
        fn new(maximize: bool) -> Self {
            Self {
                maximize,
                samples: 0,
                best: None,
                witness: String::new(),
            }
        }

        fn record(&mut self, value: f64, witness: impl FnOnce() -> String) {
            if !value.is_finite() {
                return;
            }
            self.samples += 1;
            let better = match self.best {
                None => true,
                Some(current) => {
                    if self.maximize {
                        value > current
                    } else {
                        value < current
                    }
                }
            };
            if better {
                self.best = Some(value);
                self.witness = witness();
            }
        }

        /// 零样本必须**响亮地**报出来, 不能静默给个初值.
        fn report(&self, label: &str) {
            match self.best {
                Some(value) => println!(
                    "  {label}: {value:.4e}   (样本 {} 条; 出处 {})",
                    self.samples, self.witness
                ),
                None => println!("  {label}: ⚠ 该路径 0 样本 —— 没有测到, 不是测出来是 0",),
            }
        }
    }

    /// 一次求解在某条带界指标行上留下的量级.
    ///
    /// `residual` 是 `Option`: 解落在执行界内侧时**没有残差可言**, 而"量级之比"是
    /// 这一行自己的性质, 跟残差有没有无关 —— 两者的样本集不同, 分开记, 免得又出现
    /// "某个统计量其实没测到几条"却看不出来的情况.
    struct RowMeasurement {
        residual: Option<f64>,
        check_magnitude: f64,
        lp_magnitude: f64,
        /// 这一行自己的合同界 —— 不是本轮扫到的那个值. 出处要能自证, 否则会出现
        /// "G 界=6.00" 这种一看就不可能的标注.
        own_bound: f64,
    }

    /// 从公开结果里恢复按煤池顺序排列的配比.
    ///
    /// `orders` 已按 `OUTPUT_RATIO_TOLERANCE`(1e-5) 滤掉了微量煤, 所以 `Σ|aᵢxᵢ|`
    /// 会略微偏小; 31 煤池上界约 n×1e-5×max|aᵢ| ≈ 3e-3, 相对量级 ~5 是 0.06%,
    /// 对我们只取两三位有效数字的比值无影响.
    fn ratios_of(result: &BlendResult, coals: &[Coal]) -> Vec<f64> {
        coals
            .iter()
            .map(|coal| {
                result
                    .orders
                    .iter()
                    .find(|order| order.coal == coal.name)
                    .map_or(0.0, |order| order.ratio)
            })
            .collect()
    }

    /// 用**真正进 LP 的那一行**算量级, 不做手推近似: 线性 / 仿射 / 回归三种公式
    /// 都经由 `upper_constraint`/`lower_constraint` 编译成同一形式.
    fn measure_row(
        result: &BlendResult,
        coals: &[Coal],
        models: &EvaluatorSet,
        indicator: &str,
    ) -> Option<RowMeasurement> {
        let check = result
            .indicator_check
            .iter()
            .find(|check| check.indicator == indicator)?;
        let slack = check.slack?;
        let executed = check.value + slack;
        let upper = check.max.is_some();

        let borrowed: Vec<&Coal> = coals.iter().collect();
        let formulas = build_formulas(&borrowed, models);
        let formula = formulas.get(indicator)?;
        let (row, bound) = if upper {
            formula.upper_constraint(executed)
        } else {
            formula.lower_constraint(executed)
        };
        let ratios = ratios_of(result, coals);
        let lp_magnitude: f64 = row
            .iter()
            .zip(&ratios)
            .map(|(coefficient, ratio)| (coefficient * ratio).abs())
            .sum::<f64>()
            .max(bound.abs());

        Some(RowMeasurement {
            // 只有落在执行界外侧的行才有残差.
            residual: (slack < 0.0).then(|| -slack),
            check_magnitude: check.value.abs().max(executed.abs()),
            lp_magnitude,
            own_bound: check.max.or(check.min).unwrap_or(f64::NAN),
        })
    }

    struct Accumulators {
        /// residual/(1+复核量级) 的最大值 —— ACCEPT_TOLERANCE 的余量看它.
        accept_ratio: Extremum,
        /// 复核量级/LP 行量级 的最小值 —— "复核会不会比 LP 严"看它.
        magnitude_ratio: Extremum,
        /// residual/(1+LP 行量级) 的最大值 —— FEASIBILITY_TOLERANCE 的余量看它.
        lp_ratio: Extremum,
        /// 放行量换算到指标单位的最大值 —— 与化验 0.01 分辨率对比看它.
        allowance: Extremum,
        /// 同上, 但只算实测量级落在物理量程内 (≤100) 的行.
        ///
        /// 分开报是因为: 放行量随**实测值**的量级走, 而回归模型可能吐出 CSR=133
        /// 这种越出 0~100 的数 (代码已就此告警并降级为未验证). 那种行的放行量最大,
        /// 却不该拿来给"合同界能被放宽多少"背书 —— 引用时用这一条.
        allowance_in_range: Extremum,
    }

    impl Accumulators {
        fn new() -> Self {
            Self {
                accept_ratio: Extremum::new(true),
                magnitude_ratio: Extremum::new(false),
                lp_ratio: Extremum::new(true),
                allowance: Extremum::new(true),
                allowance_in_range: Extremum::new(true),
            }
        }

        fn absorb(&mut self, tag: &str, indicator: &str, row: &RowMeasurement) {
            let witness = || {
                format!(
                    "{tag}/{indicator} 该行合同界={:.2} 实测量级={:.1}",
                    row.own_bound, row.check_magnitude
                )
            };
            // 量级之比与放行量: 每一条带界的行都算, 与有没有残差无关.
            self.magnitude_ratio.record(
                (1.0 + row.check_magnitude) / (1.0 + row.lp_magnitude),
                witness,
            );
            let allowance = ACCEPT_TOLERANCE * (1.0 + row.check_magnitude);
            self.allowance.record(allowance, witness);
            if row.check_magnitude <= 100.0 {
                self.allowance_in_range.record(allowance, witness);
            }
            // 残差口径: 只有落在执行界外侧的行才进样本.
            let Some(residual) = row.residual else { return };
            self.accept_ratio
                .record(residual / (1.0 + row.check_magnitude), witness);
            self.lp_ratio
                .record(residual / (1.0 + row.lp_magnitude), witness);
        }

        fn report(&self, title: &str) {
            println!("\n{title}");
            self.accept_ratio
                .report("最大 residual/(1+复核量级)  [ACCEPT_TOLERANCE 余量]");
            self.magnitude_ratio
                .report("最小 复核量级/LP 行量级     [<1 即复核比 LP 严]");
            self.lp_ratio
                .report("最大 residual/(1+LP 行量级) [FEASIBILITY_TOLERANCE 余量]");
            self.allowance
                .report("最大 放行量 (指标单位)      [含越出物理量程的模型输出]");
            self.allowance_in_range
                .report("  └ 仅物理量程内 (≤100)     [对比化验 0.01, 引用这一条]");
        }
    }

    /// 宽煤池: master 里化验齐全的那些, 价格按"好煤贵"合成 —— 用户在煤池页填价即此形状.
    ///
    /// 只用作**测量输入**, 不断言任何 master 内容: 数据更新会让下面的数字变化,
    /// 那正是要重跑本测试、并按新结果更新常量注释的信号.
    fn wide_pool() -> Vec<Coal> {
        let Ok(master) = crate::seed::CoalMaster::load_embedded() else {
            return Vec::new();
        };
        master
            .coals
            .iter()
            .filter(|entry| entry.has_full_indicators())
            .filter_map(|entry| {
                let props = &entry.props;
                let fob = 900.0 + props["G"] * 6.0 + props["CSR"] * 5.0 - props["A"] * 25.0
                    + props["Y"] * 8.0
                    - props["S"] * 60.0;
                entry.to_coal(Some(fob.max(400.0)), Some(30.0))
            })
            .collect()
    }

    fn verified_pool() -> Vec<Coal> {
        crate::seed::CoalMaster::load_embedded().map_or_else(
            |_| Vec::new(),
            |master| {
                master
                    .verified()
                    .filter_map(|entry| entry.to_coal(None, None))
                    .collect()
            },
        )
    }

    fn default_contract() -> Vec<Spec> {
        crate::seed::CoalMaster::load_embedded()
            .map_or_else(|_| Vec::new(), |master| master.default_contract.specs)
    }

    /// 五种合同变体: 裸合同 / 安全余量 / 两位小数四舍五入 / 两位小数截断 / 上下双界.
    fn variants() -> Vec<(&'static str, Option<f64>, Option<AcceptanceMode>, bool)> {
        vec![
            ("裸合同", None, None, false),
            ("margin", Some(0.3), None, false),
            ("round2", None, Some(AcceptanceMode::Round), false),
            ("trunc2", None, Some(AcceptanceMode::Truncate), false),
            ("双界", None, None, true),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn sweep(
        tag: &'static str,
        coals: &[Coal],
        base: &[Spec],
        models: &EvaluatorSet,
        step: f64,
        span: i32,
        into: &mut Accumulators,
    ) {
        if coals.is_empty() || base.is_empty() {
            return;
        }
        for (label, margin, mode, two_sided) in variants() {
            for (index, spec) in base.iter().enumerate() {
                if spec.enforcement != Enforcement::Hard {
                    continue;
                }
                let (direction, anchor) = match (spec.direction, spec.max, spec.min) {
                    (Direction::Upper, Some(maximum), _) => (Direction::Upper, maximum),
                    (Direction::Lower, _, Some(minimum)) => (Direction::Lower, minimum),
                    _ => continue,
                };
                for offset in -span..=span {
                    let candidate = anchor + f64::from(offset) * step;
                    for truncate in [false, true] {
                        let mut specs = base.to_vec();
                        match direction {
                            Direction::Upper => {
                                specs[index].max = Some(candidate);
                                if two_sided {
                                    specs[index].min = Some(candidate - 4.0);
                                    specs[index].direction = Direction::Range;
                                }
                            }
                            Direction::Lower => {
                                specs[index].min = Some(candidate);
                                if two_sided {
                                    specs[index].max = Some(candidate + 4.0);
                                    specs[index].direction = Direction::Range;
                                }
                            }
                            Direction::Range => continue,
                        }
                        for spec in &mut specs {
                            spec.margin = margin;
                            if let Some(mode) = mode {
                                spec.acceptance = Some(AcceptanceRule {
                                    mode,
                                    decimals: Some(2),
                                    tolerance: 0.0,
                                });
                            }
                        }
                        let request = BlendRequest {
                            coals: coals.to_vec(),
                            specs: specs.clone(),
                            total_quantity: Some(3_700.0),
                            truncate_decimal: truncate,
                        };
                        let result = solve_with_evaluators(&request, models);
                        if !result.ok {
                            continue;
                        }
                        for spec in specs
                            .iter()
                            .filter(|spec| spec.enabled && spec.enforcement == Enforcement::Hard)
                        {
                            if let Some(row) = measure_row(&result, coals, models, &spec.indicator)
                            {
                                into.absorb(&format!("{tag}·{label}"), &spec.indicator, &row);
                            }
                        }
                    }
                }
            }
        }
    }

    /// 训练一组能过门控的 G 仿射 + CSR 回归评估器, 把 `AffineCalibration` /
    /// `Regression` 那条路也拉进测量 —— 它今天不在产品路径上 (`solve` 用空评估器),
    /// 但 `solve_with_evaluators` 是公开 API, `predict.rs` 就是为接上它写的.
    fn trained_evaluators() -> EvaluatorSet {
        let g_observations: Vec<GObservation> = (0..40)
            .map(|index| {
                let g_linear = 60.0 + f64::from(index) * 0.8;
                GObservation {
                    g_linear,
                    g_measured: 0.9 * g_linear + 4.0,
                }
            })
            .collect();
        let csr_observations: Vec<CsrObservation> = (0..40)
            .map(|index| {
                let t = f64::from(index);
                let s = 0.5 + (t * 0.7).sin().abs() * 3.0;
                let a = 6.0 + (t * 1.3).cos().abs() * 8.0;
                let v = 18.0 + (t * 0.5 + 1.0).sin().abs() * 16.0;
                let g = 65.0 + (t * 0.9).cos().abs() * 28.0;
                let y = 9.0 + (t * 1.7).sin().abs() * 11.0;
                let m = 8.0 + (t * 0.3 + 0.5).cos().abs() * 16.0;
                CsrObservation {
                    s,
                    a,
                    v,
                    g,
                    y,
                    m,
                    csr_measured: 30.0 + s + 0.5 * a + 0.8 * v + 0.3 * g + 0.6 * y + 0.4 * m,
                }
            })
            .collect();
        let policy = ModelPolicy {
            min_g_samples: 8,
            min_csr_samples: 8,
            max_g_cv_mae: 5.0,
            max_csr_cv_mae: 5.0,
            extrapolation_ratio: 1.0,
            ..ModelPolicy::default()
        };
        EvaluatorSet::train(&g_observations, &csr_observations, &policy)
            .expect("测量用评估器应能训练")
    }

    #[test]
    #[ignore = "量级测量, 手动触发: cargo test --release -- --ignored --nocapture"]
    fn measure_tolerance_headroom() {
        println!(
            "\n常量现值: FEASIBILITY_TOLERANCE = {FEASIBILITY_TOLERANCE:e}, \
             ACCEPT_TOLERANCE = {ACCEPT_TOLERANCE:e}"
        );
        println!("化验分辨率按 0.01 计.");

        let verified = verified_pool();
        let wide = wide_pool();
        let real = super::tests::real_case_coals();
        let contract = default_contract();
        let real_contract = vec![
            Spec::upper("A", 10.0),
            Spec::upper("S", 1.0),
            Spec::upper("V", 28.0),
            Spec::lower("Y", 15.0),
            Spec::lower("G", 85.0),
        ];

        // —— 线性行: 今天产品路径上的全部 Hard 行 (空评估器).
        let plain = EvaluatorSet::default();
        let mut linear = Accumulators::new();
        sweep(
            "verified4",
            &verified,
            &contract,
            &plain,
            0.01,
            200,
            &mut linear,
        );
        sweep(
            "real4",
            &real,
            &real_contract,
            &plain,
            0.01,
            200,
            &mut linear,
        );
        sweep("wide", &wide, &contract, &plain, 0.03, 120, &mut linear);
        linear.report("【线性行】formula_for 的加权平均 —— 今天 solve() 走的就是这条");

        // —— 仿射 / 回归行: G 的 AffineCalibration 与 CSR 的 Regression.
        let models = trained_evaluators();
        println!(
            "\n评估器状态: G 校准 {}, CSR 回归 {}",
            if models.g.is_some() {
                "已启用"
            } else {
                "未启用"
            },
            if models.csr.is_some() {
                "已启用"
            } else {
                "未启用"
            },
        );
        let mut modelled = Accumulators::new();
        sweep(
            "verified4",
            &verified,
            &contract,
            &models,
            0.01,
            200,
            &mut modelled,
        );
        sweep("wide", &wide, &contract, &models, 0.03, 120, &mut modelled);
        modelled.report("【仿射/回归行】solve_with_evaluators —— predict.rs 接上后即产品路径");

        println!("\n对照: 放行量 / 化验 0.01 —— 这个比值越小越安全, 它是合同界被放宽的真实幅度.");

        // 断言只管"测量确实跑到了", 不断言任何 master 内容.
        assert!(
            linear.accept_ratio.samples > 0,
            "线性行 0 样本: 网格没解出任何顶界的解, 测量没跑到"
        );
    }
}
