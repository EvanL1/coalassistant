//! 煤阶交互罚项 (配煤不相容): CSR配 = Σxᵢ·CSRᵢ − k·D.
//!
//! D = 配比加权的煤阶方差 Σxᵢ(Rᵢ − R̄)², R 为镜质组反射率 Ro. 单煤 D = 0, 两种煤煤阶
//! 差得越远、比例越接近对半 D 越大. 等价于混料二次模型里 βᵢⱼ = −k·(Rᵢ − Rⱼ)² 的交互项.
//!
//! D 是二次的, LP 不能直接放. 但方差是对中心的最小二乘: 对任意 c,
//! Σxᵢ(Rᵢ − c)² = D + (R̄ − c)² ≥ D. 所以把每种煤的 CSR 换成 CSRᵢ − k(Rᵢ − c)² 之后,
//! 线性加权值永远不高于真实值 —— LP 按它达标, 真实 CSR 一定达标. 每轮取 c = 上轮解的 R̄:
//! 上轮解在新 c 下线性值恰等于真实值, 仍可行, 所以成本只降不升. 线性值比真实值少算
//! k(R̄ − c)², c 收敛到 R̄ 时归零, 体检里的 CSR 就是真实值.
//!
//! 这个保守性只对"差的平方"成立; 改成绝对差或阈值形式就不再是下界, 别改.

use crate::model::{BlendRequest, BlendResult, Coal};
use crate::petrography::Petrography;

/// 没有煤岩数据时由挥发换算煤阶: Ro ≈ 2.478 − 0.0458·Vdaf. 由 Vdaf 17–36% 的 8 种
/// 炼焦煤 (Rmax) 拟合, r = −0.98; 范围外是外推.
const RO_INTERCEPT: f64 = 2.478;
const RO_PER_VDAF: f64 = 0.0458;
/// 线性化迭代轮数上限. 配方是 LP 顶点, 一般 2~3 轮就不再变.
const MAX_ROUNDS: usize = 8;
/// 体检 CSR 允许少算的上限 (CSR 点). 低于它视为收敛.
const CONVERGED: f64 = 1e-9;

/// 一种煤的煤阶: 有效煤岩数据的反射率均值优先, 否则由挥发换算.
fn coal_rank(coal: &Coal) -> Option<f64> {
    coal.petrography
        .as_ref()
        .filter(|data| data.is_valid())
        .and_then(Petrography::mean_std)
        .map(|(mean, _)| mean)
        .or_else(|| coal.get("V").map(|vdaf| RO_INTERCEPT - RO_PER_VDAF * vdaf))
}

/// 求解 (或验算) 并计入煤阶交互. `inner` 是不带交互的原求解流程.
pub(crate) fn solve(
    request: &BlendRequest,
    csr_regression_active: bool,
    inner: impl Fn(&BlendRequest) -> BlendResult,
) -> BlendResult {
    let mut plain = request.clone();
    let Some(k) = plain.rank_interaction.take().map(|setting| setting.k) else {
        return inner(&plain);
    };
    if k == 0.0 {
        return inner(&plain);
    }
    let skip = |reason: String| {
        let mut result = inner(&plain);
        result
            .warnings
            .push(format!("煤阶交互本次不启用: {reason}"));
        result
    };
    if !k.is_finite() || k < 0.0 {
        return skip(format!("k = {k} 无效, 须为非负数"));
    }
    if csr_regression_active {
        return skip("CSR 已由实测回归模型预测, 不再叠加".into());
    }
    let ranks: Vec<Option<f64>> = plain.coals.iter().map(coal_rank).collect();
    let missing: Vec<&str> = plain
        .coals
        .iter()
        .zip(&ranks)
        .filter(|(_, rank)| rank.is_none())
        .map(|(coal, _)| coal.name.as_str())
        .collect();
    if !missing.is_empty() {
        return skip(format!(
            "{} 缺挥发和煤岩数据, 算不出煤阶",
            missing.join("/")
        ));
    }
    let ranks: Vec<f64> = ranks.into_iter().flatten().collect();

    let mut center = ranks.iter().sum::<f64>() / ranks.len() as f64;
    let mut last: Option<(BlendResult, f64)> = None;
    for _ in 0..MAX_ROUNDS {
        let result = inner(&with_center(&plain, &ranks, k, center));
        if !result.ok {
            // 第一轮无解 = 真不可行; 之后无解只可能是数值问题, 退回上一轮的可行解.
            return match last {
                Some((mut previous, _)) => {
                    previous
                        .warnings
                        .push("煤阶交互迭代中途无解, 已保留上一轮配方".into());
                    previous
                }
                None => result,
            };
        }
        let (mean, variance) = blend_moments(&result, &plain.coals, &ranks);
        let shortfall = k * (mean - center).powi(2);
        let mut result = result;
        result.rank_variance = Some(variance);
        result.csr_interaction_penalty = Some(k * variance);
        if shortfall <= CONVERGED {
            return result;
        }
        center = mean;
        last = Some((result, shortfall));
    }
    let (mut result, shortfall) = last.expect("MAX_ROUNDS > 0");
    result.warnings.push(format!(
        "煤阶交互迭代未收敛, 体检 CSR 为保守值 (最多少算 {shortfall:.2})"
    ));
    result
}

/// 以中心 c 线性化: 每种煤的 CSR 换成 CSRᵢ − k(Rᵢ − c)².
fn with_center(plain: &BlendRequest, ranks: &[f64], k: f64, center: f64) -> BlendRequest {
    let mut request = plain.clone();
    for (coal, rank) in request.coals.iter_mut().zip(ranks) {
        if let Some(csr) = coal.get("CSR") {
            coal.props
                .insert("CSR".into(), csr - k * (rank - center).powi(2));
        }
    }
    request
}

/// 结果配方的煤阶均值与方差 (按配比归一).
fn blend_moments(result: &BlendResult, coals: &[Coal], ranks: &[f64]) -> (f64, f64) {
    let parts: Vec<(f64, f64)> = coals
        .iter()
        .zip(ranks)
        .filter_map(|(coal, rank)| result.recipe.get(&coal.name).map(|x| (*x, *rank)))
        .collect();
    let total: f64 = parts.iter().map(|(x, _)| x).sum();
    if total <= 0.0 {
        return (0.0, 0.0);
    }
    let mean = parts.iter().map(|(x, r)| x * r).sum::<f64>() / total;
    let variance = parts
        .iter()
        .map(|(x, r)| x * (r - mean).powi(2))
        .sum::<f64>()
        / total;
    (mean, variance)
}
