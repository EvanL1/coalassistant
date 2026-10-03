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

use std::collections::HashMap;

use crate::model::{BlendRequest, BlendResult, Coal, Direction};
use crate::petrography::Petrography;

/// 没有煤岩数据时由挥发换算煤阶: Ro ≈ 2.478 − 0.0458·Vdaf. 由 Vdaf 17–36% 的 8 种
/// 炼焦煤 (Rmax) 拟合, r = −0.98; 范围外是外推.
const RO_INTERCEPT: f64 = 2.478;
const RO_PER_VDAF: f64 = 0.0458;
/// 线性化迭代轮数上限. 配方是 LP 顶点, 一般 2~3 轮就不再变.
const MAX_ROUNDS: usize = 8;
/// 体检 CSR 允许少算的上限 (CSR 点). 低于它视为收敛.
const CONVERGED: f64 = 1e-9;
/// 初始中心无解时试探中心的步长 (Ro %). k = 66 时离真实中心半步最多少算 0.007 点.
const SCAN_STEP: f64 = 0.02;
/// 试探中心的格点数上限 (不含各煤自身煤阶).
const MAX_SCAN: usize = 24;

/// 一种煤的煤阶: 有效煤岩数据的反射率均值优先, 否则由挥发换算.
fn coal_rank(coal: &Coal) -> Option<f64> {
    coal.petrography
        .as_ref()
        .filter(|data| data.is_valid())
        .and_then(Petrography::mean_std)
        .map(|(mean, _)| mean)
        .or_else(|| coal.get("V").map(|vdaf| RO_INTERCEPT - RO_PER_VDAF * vdaf))
}

/// 求解 (或验算) 并计入煤阶交互. `inner(request, csr_shift, diagnose)` 是不带交互的原求解
/// 流程: `csr_shift` 为各煤 CSR 在公式里要扣的点数 (不改煤的化验值); `diagnose` 为 false
/// 时无解不跑逐项诊断 (试探中心时用, 免得每次都诊断一遍).
pub(crate) fn solve(
    request: &BlendRequest,
    csr_regression_active: bool,
    inner: impl Fn(&BlendRequest, Option<&HashMap<String, f64>>, bool) -> BlendResult,
) -> BlendResult {
    let mut plain = request.clone();
    let Some(k) = plain.rank_interaction.take().map(|setting| setting.k) else {
        return inner(&plain, None, true);
    };
    if k == 0.0 {
        return inner(&plain, None, true);
    }
    let skip = |reason: String| {
        let mut result = inner(&plain, None, true);
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
    // 线性值 ≤ 真实值只对下限保守; 上限反过来会放过真实超标的配方.
    if plain.specs.iter().any(|spec| {
        spec.enabled
            && spec.indicator == "CSR"
            && matches!(spec.direction, Direction::Upper | Direction::Range)
    }) {
        return skip("CSR 合同有上限, 线性化在上限方向不保守".into());
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

    // 罚项只会让 CSR 更低: 不计交互都无解就一定真无解, 原样返回 (含逐项诊断).
    let unpenalized = inner(&plain, None, true);
    if !unpenalized.ok {
        return unpenalized;
    }
    if !unpenalized
        .indicator_check
        .iter()
        .any(|check| check.indicator == "CSR")
    {
        // 有煤缺 CSR 时没有 CSR 公式, 扣分无处体现; 不报罚项字段, 免得界面误导.
        return unpenalized;
    }
    let linearized =
        |center: f64| inner(&plain, Some(&csr_shift(&plain, &ranks, k, center)), false);
    let mut center = blend_moments(&unpenalized, &plain.coals, &ranks).0;
    let mut result = linearized(center);
    if !result.ok {
        // 线性化只在 c 附近精确, 可行解离 c 远时会被保守估计误杀. 换中心再试.
        match scan_centers(&ranks, &linearized) {
            Some((found_center, found)) => {
                center = found_center;
                result = found;
            }
            None => {
                let mut warnings = unpenalized.warnings;
                warnings.push("不计煤阶交互有解, 计入后找不到达标配方".into());
                return BlendResult::infeasible("计入煤阶交互后无可行配方", warnings);
            }
        }
    }
    let mut round = 1;
    loop {
        let (mean, variance) = blend_moments(&result, &plain.coals, &ranks);
        result.rank_variance = Some(variance);
        result.csr_interaction_penalty = Some(k * variance);
        let shortfall = k * (mean - center).powi(2);
        if shortfall <= CONVERGED {
            return result;
        }
        if round == MAX_ROUNDS {
            result.warnings.push(format!(
                "煤阶交互迭代未收敛, 体检 CSR 为保守值 (最多少算 {shortfall:.2})"
            ));
            return result;
        }
        // 上一轮的解在新中心下仍可行 (线性值 = 真实值), 所以这里理论上不会无解.
        let next = linearized(mean);
        if !next.ok {
            result.warnings.push(format!(
                "煤阶交互迭代中途无解, 已保留上一轮配方, 体检 CSR 为保守值 (最多少算 {shortfall:.2})"
            ));
            return result;
        }
        center = mean;
        result = next;
        round += 1;
    }
}

/// 在最低到最高煤阶之间按 [`SCAN_STEP`] 试探中心, 再加上每种煤自身的煤阶
/// (单用一种煤时以它为中心线性化恰好精确), 返回最便宜的可行解.
fn scan_centers(
    ranks: &[f64],
    linearized: &impl Fn(f64) -> BlendResult,
) -> Option<(f64, BlendResult)> {
    let low = ranks.iter().copied().fold(f64::INFINITY, f64::min);
    let high = ranks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // 步长至少 SCAN_STEP, 且最多试 MAX_SCAN + 1 个格点 + 各煤自身煤阶, 控制单次求解耗时.
    let steps = ((high - low) / SCAN_STEP).ceil().min(MAX_SCAN as f64) as usize;
    let grid = (0..=steps).map(|i| low + (high - low) * i as f64 / steps.max(1) as f64);
    let net = |result: &BlendResult| {
        result
            .cost
            .as_ref()
            .map_or(f64::INFINITY, |cost| cost.net_per_ton)
    };
    grid.chain(ranks.iter().copied())
        .map(|center| (center, linearized(center)))
        .filter(|(_, result)| result.ok)
        .min_by(|a, b| net(&a.1).total_cmp(&net(&b.1)))
}

/// 以中心 c 线性化: 每种煤的 CSR 在公式里扣 k(Rᵢ − c)².
fn csr_shift(plain: &BlendRequest, ranks: &[f64], k: f64, center: f64) -> HashMap<String, f64> {
    plain
        .coals
        .iter()
        .zip(ranks)
        .map(|(coal, rank)| (coal.name.clone(), k * (rank - center).powi(2)))
        .collect()
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
