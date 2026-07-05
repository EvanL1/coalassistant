//! LP 求解器, 基于 Clarabel.
//!
//! 模型:
//!   决策变量: x_i ∈ [0, 1], i = 1..n
//!   目标:    min Σ cif(i) · x_i
//!   约束:    Σ x_i = 1
//!            Σ ind_i · x_i ≤ max  (上限约束)
//!            Σ ind_i · x_i ≥ min  (下限约束)
//!            x_i ≥ 0
//!
//! 8 项指标默认按线性加权处理. CSR 可选: 请求带历史观测时, 先用线性回归预测覆盖
//! 各煤 CSR 再建 LP (见 `apply_csr_prediction`); 不做 σ(Ro) 迭代.
//! 如需其他派生指标, 在调用前对 Coal.props 进行预计算即可.
use crate::model::*;
use crate::petrography::{self, Petrography, NOTCH_WINDOW};
use crate::predict::{CsrObservation, CsrPredictor};
use clarabel::algebra::CscMatrix;
use clarabel::solver::*;
use std::collections::HashSet;

const EPS_TRUNCATE: f64 = 0.0999;
const BINDING_TOL: f64 = 0.05;
/// CSR 回归拟合质量门槛: R² 低于此值视为不可信, 回退录入 CSR.
/// 0.6 = 至少解释 60% 方差; 偏保守, 想更严就调高 (如 0.8).
const MIN_CSR_R2: f64 = 0.6;
/// 岩相 σ 超标时线性代理收紧的最大迭代轮数.
const MAX_PETRO_REFINE: usize = 4;
/// 收紧步进的保守系数, 避免渐进擦边不收敛.
const PETRO_SHRINK_SAFETY: f64 = 0.999;
/// σ 达标判定容差.
const PETRO_SIGMA_TOL: f64 = 1e-6;

/// 主求解函数.
///
/// 岩相非线性处理 (调研 2026-07-04 §5 阶段一方案):
/// LP 的 petro 约束是线性代理 (Σx·σ_j), 会系统性低估混煤真实 σ (μ 离散贡献).
/// 参配煤全带煤岩直方图时, 求解后按全方差定律精确复验 σ; 超标则按违约比收紧
/// 代理上限重解 (最多 MAX_PETRO_REFINE 轮). 代理无杠杆 (收紧至不可行) 时回退
/// 最后可行解并显式警告 —— 违约永不静默, 但也不把合同层面可行的方案变成不可行.
/// 可靠的自动满足需阶段二信赖域 SLP, 见调研文档.
pub fn solve(req: &BlendRequest) -> BlendResult {
    let active_specs: Vec<&Spec> = req.specs.iter().filter(|s| s.enabled).collect();
    let eps = if req.truncate_decimal {
        EPS_TRUNCATE
    } else {
        0.0
    };

    // 可选 CSR 预测: 有历史观测就拟合线性回归覆盖各煤 CSR (拟合失败时附警告并回退).
    let (mut coals, mut warnings) =
        apply_csr_prediction(&req.coals, req.csr_observations.as_deref());

    // 煤岩补齐: 有直方图但缺 petro 标量的煤, 用单煤直方图 σ 补上,
    // 让带煤岩数据的煤不因缺标量被剔除.
    for c in &mut coals {
        if !c.has("petro") {
            if let Some(p) = &c.petrography {
                if let Some((_, std)) = petrography::hist_mean_std(&p.hist) {
                    c.props.insert("petro".into(), std);
                }
            }
        }
    }

    // 容错: 剔除缺关键指标的煤
    let required: HashSet<String> = active_specs.iter().map(|s| s.indicator.clone()).collect();
    let mut kept: Vec<&Coal> = Vec::new();
    for c in &coals {
        let missing: Vec<&String> = required.iter().filter(|k| !c.has(k)).collect();
        if missing.is_empty() {
            kept.push(c);
        } else {
            warnings.push(format!(
                "剔除 {}: 缺指标 {}",
                c.name,
                missing
                    .iter()
                    .map(|k| label_zh(k))
                    .collect::<Vec<_>>()
                    .join("/")
            ));
        }
    }

    if kept.is_empty() {
        return BlendResult::infeasible("无可用煤", warnings);
    }

    // petro 上限 (用于精确 σ 校验与代理收紧). 只看 max 侧, σ 无下限语义.
    let petro_max: Option<f64> = active_specs
        .iter()
        .find(|s| {
            s.indicator == "petro" && matches!(s.direction, Direction::Upper | Direction::Range)
        })
        .and_then(|s| s.max);

    let mut petro_cap: Option<f64> = None; // 收紧后的代理上限 (None = 用合同原值)
    let mut refine_count = 0usize;
    let mut fallback: Option<BlendResult> = None; // 收紧前的最后可行解

    loop {
        let (mut result, x) = match solve_once(
            &kept,
            &active_specs,
            req,
            eps,
            petro_cap,
            warnings.clone(),
        ) {
            Some(v) => v,
            None => {
                return match fallback {
                    Some(mut r) => {
                        r.warnings.push(
                                "岩相校验: 线性代理收紧后 LP 不可行, 已回退收紧前方案 (σ 仍超标, 见岩相警告); 建议停用高离散/凹口煤或放宽岩相上限"
                                    .into(),
                            );
                        r
                    }
                    None => BlendResult::infeasible("约束冲突, LP 不可行", warnings),
                };
            }
        };

        // 岩相精确校验: 参配煤 (x > 1e-5) 全带**有效**煤岩数据才可计算.
        // 空直方图/全零频率/镜质组含量 0 视同缺数据 (否则会静默算出只覆盖部分煤的 σ).
        let participating: Vec<(&Coal, f64)> = kept
            .iter()
            .zip(x.iter())
            .filter(|(_, &xi)| xi > 1e-5)
            .map(|(c, &xi)| (*c, xi))
            .collect();
        let parts: Vec<(&Petrography, f64)> = participating
            .iter()
            .filter_map(|(c, xi)| {
                c.petrography
                    .as_ref()
                    .filter(|p| p.is_valid())
                    .map(|p| (p, *xi))
            })
            .collect();

        if parts.is_empty() {
            // 没有任何参配煤带煤岩数据 = 功能未启用, 静默跳过
            // (不产生用户无法消除的常驻警告; 当前 master 数据尚无煤岩字段)
            return result;
        }
        if parts.len() < participating.len() {
            if petro_max.is_some() {
                let missing: Vec<&str> = participating
                    .iter()
                    .filter(|(c, _)| !c.petrography.as_ref().is_some_and(|p| p.is_valid()))
                    .map(|(c, _)| c.name.as_str())
                    .collect();
                result.warnings.push(format!(
                    "岩相校验跳过: {} 缺有效煤岩直方图, σ 仅按线性代理约束 (会低估混煤离散度)",
                    missing.join("/")
                ));
            }
            return result;
        }

        let sigma_data = petrography::mix_histogram(&parts)
            .and_then(|mixed| petrography::hist_mean_std(&mixed).map(|ms| (mixed, ms)));
        let Some((mixed, (mean, sigma))) = sigma_data else {
            if petro_max.is_some() {
                result
                    .warnings
                    .push("岩相校验跳过: 煤岩数据无效 (直方图/镜质组含量为空)".into());
            }
            return result;
        };

        let notch = petrography::detect_notch(&mixed, NOTCH_WINDOW.0, NOTCH_WINDOW.1);
        if let Some(n) = &notch {
            result.warnings.push(format!(
                "岩相: 混煤反射率分布在 {:.1}~{:.1} 主焦区间存在凹口 (谷深比 {:.2}), 可能损害焦炭热强度",
                NOTCH_WINDOW.0, NOTCH_WINDOW.1, n.depth_ratio
            ));
        }

        let sigma_ok = petro_max.map(|m| sigma <= m + PETRO_SIGMA_TOL);
        result.petrography_check = Some(PetrographyCheck {
            mean,
            sigma,
            sigma_max: petro_max,
            sigma_ok,
            notch,
            refine_iterations: refine_count,
        });

        match sigma_ok {
            // σ 超标且还有迭代额度 → 按违约比平方收紧代理上限重解
            Some(false) if refine_count < MAX_PETRO_REFINE => {
                let max = petro_max.unwrap();
                let cur = petro_cap.unwrap_or(max);
                petro_cap = Some(cur * (max / sigma).powi(2) * PETRO_SHRINK_SAFETY);
                refine_count += 1;
                fallback = Some(result);
            }
            // 迭代用尽仍超标 → 如实报告, 不静默
            Some(false) => {
                result.warnings.push(format!(
                    "岩相校验: 实际 σ={:.3} 超上限 {:.3} (线性代理低估), 自动收紧 {} 轮未收敛; 建议停用高离散/凹口煤或放宽岩相上限",
                    sigma,
                    petro_max.unwrap(),
                    refine_count
                ));
                return result;
            }
            // 达标 / 无 petro 约束 (校验信息仍挂上)
            _ => return result,
        }
    }
}

/// 单次 LP 求解 + 三视图后处理. LP 不可行 → None.
/// petro_cap: 岩相代理收紧迭代传入的替代上限 (替换 petro spec 的合同 max).
fn solve_once(
    kept: &[&Coal],
    active_specs: &[&Spec],
    req: &BlendRequest,
    eps: f64,
    petro_cap: Option<f64>,
    warnings: Vec<String>,
) -> Option<(BlendResult, Vec<f64>)> {
    let n = kept.len();
    let cifs: Vec<f64> = kept.iter().map(|c| c.cif()).collect();

    // 构造不等式: A_ub · x ≤ b_ub
    // direction 决定哪一侧约束生效:
    //   Upper → 只看 max (越低越好)
    //   Lower → 只看 min (越高越好)
    //   Range → min 和 max 都看
    // margin 安全余量: 上限减 margin、下限加 margin (只影响 LP, 展示层仍用合同原界限).
    let mut a_ub: Vec<Vec<f64>> = Vec::new();
    let mut b_ub: Vec<f64> = Vec::new();

    for spec in active_specs {
        let coefs: Vec<f64> = kept
            .iter()
            .map(|c| c.get(&spec.indicator).unwrap())
            .collect();
        let use_max = matches!(spec.direction, Direction::Upper | Direction::Range);
        let use_min = matches!(spec.direction, Direction::Lower | Direction::Range);
        // 负 margin 会反向放宽约束产出违约配比, 在此钳制 (solve_json 是对外 JSON 边界)
        let margin = spec.margin.unwrap_or(0.0).max(0.0);
        if use_max {
            if let Some(max) = spec.max {
                let max = if spec.indicator == "petro" {
                    petro_cap.unwrap_or(max)
                } else {
                    max
                };
                a_ub.push(coefs.clone());
                b_ub.push(max + spec_eps(spec, eps) - margin);
            }
        }
        if use_min {
            if let Some(min) = spec.min {
                a_ub.push(coefs.iter().map(|v| -v).collect());
                b_ub.push(-(min + margin));
            }
        }
    }

    let lp = LpProblem {
        n,
        c: cifs.clone(),
        a_ub,
        b_ub,
    };

    let (x, _obj) = lp.solve()?;

    // 后处理: 三视图
    let recipe: std::collections::HashMap<String, f64> = kept
        .iter()
        .zip(x.iter())
        .filter(|(_, &xi)| xi > 1e-5)
        .map(|(c, &xi)| (c.name.clone(), xi))
        .collect();

    let fob_per_ton: f64 = kept.iter().zip(x.iter()).map(|(c, xi)| c.fob * xi).sum();
    let frt_per_ton: f64 = kept.iter().zip(x.iter()).map(|(c, xi)| c.frt * xi).sum();
    let cif_per_ton = fob_per_ton + frt_per_ton;

    let cost = CostBreakdown {
        fob_per_ton,
        frt_per_ton,
        cif_per_ton,
        total_fob: req.total_quantity.map(|q| q * fob_per_ton),
        total_frt: req.total_quantity.map(|q| q * frt_per_ton),
        total_cif: req.total_quantity.map(|q| q * cif_per_ton),
    };

    // 视图 B: 订单 (按配比降序)
    let mut orders: Vec<OrderItem> = kept
        .iter()
        .zip(x.iter())
        .filter(|(_, &xi)| xi > 1e-5)
        .map(|(c, &xi)| {
            let tons = req.total_quantity.map(|q| q * xi);
            OrderItem {
                coal: c.name.clone(),
                ratio: xi,
                tons,
                fob_amount: tons.map(|t| t * c.fob),
                frt_amount: tons.map(|t| t * c.frt),
                cif_amount: tons.map(|t| t * c.cif()),
            }
        })
        .collect();
    orders.sort_by(|a, b| b.ratio.partial_cmp(&a.ratio).unwrap());

    // 视图 C: 指标体检 (按 INDICATORS 顺序)
    let mut indicator_check = Vec::new();
    for &ind in INDICATORS.iter() {
        let spec = active_specs.iter().find(|s| s.indicator == ind).cloned();
        if !kept.iter().all(|c| c.has(ind)) {
            // 煤池数据不全, 跳过
            continue;
        }
        let value: f64 = kept
            .iter()
            .zip(x.iter())
            .map(|(c, xi)| c.get(ind).unwrap() * xi)
            .sum();

        let (slack, binding) = if let Some(s) = &spec {
            compute_slack_binding(value, s, spec_eps(s, eps))
        } else {
            (None, false)
        };

        indicator_check.push(IndicatorCheck {
            indicator: ind.into(),
            label_zh: label_zh(ind).into(),
            value,
            min: spec.as_ref().and_then(|s| s.min),
            max: spec.as_ref().and_then(|s| s.max),
            slack,
            binding,
        });
    }

    let result = BlendResult {
        ok: true,
        reason: None,
        recipe,
        cost: Some(cost),
        orders,
        indicator_check,
        petrography_check: None,
        warnings,
    };
    Some((result, x))
}

/// 可选 CSR 预测预处理.
///
/// 提供历史观测、样本足够且拟合 R² ≥ `MIN_CSR_R2` 时, 用预测值覆盖每只煤的 CSR;
/// 6 项自变量 (S/A/V/G/Y/M) 缺任意一项的煤保留原 CSR 并逐煤点名警告.
/// 观测缺失或为空 (None / Some([])) → 原样返回;
/// 样本不足 / 矩阵奇异 / R² 不足 → 原样返回并附警告 (不静默吞掉).
fn apply_csr_prediction(
    coals: &[Coal],
    observations: Option<&[CsrObservation]>,
) -> (Vec<Coal>, Vec<String>) {
    let obs = match observations {
        Some(o) if !o.is_empty() => o,
        _ => return (coals.to_vec(), Vec::new()),
    };
    let predictor = match CsrPredictor::fit(obs) {
        Ok(p) if p.r_squared >= MIN_CSR_R2 => p,
        Ok(p) => {
            let msg = format!(
                "CSR 预测跳过: R²={:.3} < {:.2}, 拟合质量不足",
                p.r_squared, MIN_CSR_R2
            );
            return (coals.to_vec(), vec![msg]);
        }
        Err(e) => return (coals.to_vec(), vec![format!("CSR 预测跳过: {}", e)]),
    };
    let mut warnings = Vec::new();
    let out = coals
        .iter()
        .map(|c| {
            let mut c = c.clone();
            match predictor.predict_coal(&c) {
                Some(csr) => {
                    c.props.insert("CSR".into(), csr);
                }
                None => warnings.push(format!("{}: 缺输入指标, CSR 保留录入值", c.name)),
            }
            c
        })
        .collect();
    (out, warnings)
}

/// 指标各自适用的截断容差: 一位小数截断规则只适用于百分比量纲的化验指标,
/// 岩相 σ 是两位小数量纲, eps=0.0999 会把 0.15 上限实际放宽到 0.2499,
/// 并使收紧迭代永远够不到目标 (有效界下限被 eps 托住).
fn spec_eps(spec: &Spec, eps: f64) -> f64 {
    if spec.indicator == "petro" {
        0.0
    } else {
        eps
    }
}

/// 计算单项指标的余量和是否 binding.
/// eps 是截断容差: 启用一位小数截断时 max 实际上限是 max+eps, 算 slack 要带上.
/// 只对 direction 实际启用的侧计算 slack, 保持与 LP 建模一致.
/// binding 要求 slack >= 0 (违反约束的负 slack 不算 binding).
fn compute_slack_binding(value: f64, spec: &Spec, eps: f64) -> (Option<f64>, bool) {
    let use_max = matches!(spec.direction, Direction::Upper | Direction::Range);
    let use_min = matches!(spec.direction, Direction::Lower | Direction::Range);

    let mut slacks: Vec<f64> = Vec::new();
    if use_max {
        if let Some(max) = spec.max {
            slacks.push((max + eps) - value); // 离有效上限的余量
        }
    }
    if use_min {
        if let Some(min) = spec.min {
            slacks.push(value - min); // 离下限的余量 (下限不松)
        }
    }
    if slacks.is_empty() {
        return (None, false);
    }
    let min_slack = slacks
        .iter()
        .cloned()
        .fold(f64::INFINITY, |a, b| if a < b { a } else { b });
    // binding: slack 接近 0. 容忍 LP 求解器精度产生的微小负值 (Clarabel ε ~1e-7),
    // 但显著负值 (真违反约束) 不算 binding.
    let binding = min_slack > -1e-6 && min_slack < BINDING_TOL;
    (Some(min_slack), binding)
}

// ============================================================================
// LP 子问题封装
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
        let m_ub = self.a_ub.len();
        let total_rows = 1 + m_ub + n;

        let mut triplets: Vec<(usize, usize, f64)> = Vec::new();
        // 等式 sum(x) = 1 → ZeroCone
        for j in 0..n {
            triplets.push((0, j, 1.0));
        }
        // 不等式 → NonnegativeCone
        for (i, row) in self.a_ub.iter().enumerate() {
            for (j, &v) in row.iter().enumerate() {
                triplets.push((1 + i, j, v));
            }
        }
        // 非负 x_i ≥ 0
        for j in 0..n {
            triplets.push((1 + m_ub + j, j, -1.0));
        }

        let a_csc = build_csc(total_rows, n, &triplets);
        let mut b = vec![1.0_f64];
        b.extend_from_slice(&self.b_ub);
        b.extend(std::iter::repeat_n(0.0, n));

        let p_csc = CscMatrix::<f64>::zeros((n, n));
        let cones = [ZeroConeT(1), NonnegativeConeT(m_ub + n)];

        let settings = DefaultSettingsBuilder::<f64>::default()
            .verbose(false)
            .max_iter(200)
            .build()
            .ok()?;

        let mut solver = DefaultSolver::new(&p_csc, &self.c, &a_csc, &b, &cones, settings);
        solver.solve();

        match solver.solution.status {
            SolverStatus::Solved | SolverStatus::AlmostSolved => {
                Some((solver.solution.x.clone(), solver.solution.obj_val))
            }
            _ => None,
        }
    }
}

fn build_csc(rows: usize, cols: usize, triplets: &[(usize, usize, f64)]) -> CscMatrix<f64> {
    let mut by_col: Vec<Vec<(usize, f64)>> = vec![Vec::new(); cols];
    for &(i, j, v) in triplets {
        by_col[j].push((i, v));
    }
    let mut colptr = Vec::with_capacity(cols + 1);
    let mut rowval = Vec::new();
    let mut nzval = Vec::new();
    colptr.push(0);
    for col in &mut by_col {
        col.sort_by_key(|&(i, _)| i);
        for &(i, v) in col.iter() {
            rowval.push(i);
            nzval.push(v);
        }
        colptr.push(rowval.len());
    }
    CscMatrix::new(rows, cols, colptr, rowval, nzval)
}
