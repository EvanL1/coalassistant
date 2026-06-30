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
use crate::predict::{CsrObservation, CsrPredictor};
use clarabel::algebra::CscMatrix;
use clarabel::solver::*;
use std::collections::HashSet;

const EPS_TRUNCATE: f64 = 0.0999;
const BINDING_TOL: f64 = 0.05;
/// CSR 回归拟合质量门槛: R² 低于此值视为不可信, 回退录入 CSR.
/// 0.6 = 至少解释 60% 方差; 偏保守, 想更严就调高 (如 0.8).
const MIN_CSR_R2: f64 = 0.6;

/// 主求解函数.
pub fn solve(req: &BlendRequest) -> BlendResult {
    let active_specs: Vec<&Spec> = req.specs.iter().filter(|s| s.enabled).collect();
    let eps = if req.truncate_decimal { EPS_TRUNCATE } else { 0.0 };

    // 可选 CSR 预测: 有历史观测就拟合线性回归覆盖各煤 CSR (拟合失败时附警告并回退).
    let (coals, mut warnings) = apply_csr_prediction(&req.coals, req.csr_observations.as_deref());

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
                missing.iter().map(|k| label_zh(k)).collect::<Vec<_>>().join("/")
            ));
        }
    }

    if kept.is_empty() {
        return BlendResult::infeasible("无可用煤", warnings);
    }

    let n = kept.len();
    let cifs: Vec<f64> = kept.iter().map(|c| c.cif()).collect();

    // 构造不等式: A_ub · x ≤ b_ub
    // direction 决定哪一侧约束生效:
    //   Upper → 只看 max (越低越好)
    //   Lower → 只看 min (越高越好)
    //   Range → min 和 max 都看
    let mut a_ub: Vec<Vec<f64>> = Vec::new();
    let mut b_ub: Vec<f64> = Vec::new();

    for spec in &active_specs {
        let coefs: Vec<f64> = kept.iter().map(|c| c.get(&spec.indicator).unwrap()).collect();
        let use_max = matches!(spec.direction, Direction::Upper | Direction::Range);
        let use_min = matches!(spec.direction, Direction::Lower | Direction::Range);
        if use_max {
            if let Some(max) = spec.max {
                a_ub.push(coefs.clone());
                b_ub.push(max + eps);
            }
        }
        if use_min {
            if let Some(min) = spec.min {
                a_ub.push(coefs.iter().map(|v| -v).collect());
                b_ub.push(-min);
            }
        }
    }

    let lp = LpProblem {
        n,
        c: cifs.clone(),
        a_ub,
        b_ub,
    };

    let sol = match lp.solve() {
        Some(s) => s,
        None => return BlendResult::infeasible("约束冲突, LP 不可行", warnings),
    };
    let (x, _obj) = sol;

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
            compute_slack_binding(value, s, eps)
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

    BlendResult {
        ok: true,
        reason: None,
        recipe,
        cost: Some(cost),
        orders,
        indicator_check,
        warnings,
    }
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
            let msg = format!("CSR 预测跳过: R²={:.3} < {:.2}, 拟合质量不足", p.r_squared, MIN_CSR_R2);
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
        b.extend(std::iter::repeat(0.0).take(n));

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
            SolverStatus::Solved | SolverStatus::AlmostSolved => Some((
                solver.solution.x.clone(),
                solver.solution.obj_val,
            )),
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
