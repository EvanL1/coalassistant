//! 价格稳健性: 煤价估不准时, 配方还靠不靠得住.
//!
//! 起因: 全部按估算价求解时, 最优解往往很"平" —— 很多配方的成本只差几元, 而估算价
//! 的误差有几十到几百元. 配方选哪几种煤, 很容易被估价误差而不是真实价差决定.
//!
//! 两件事, 都只靠"改价重解", 不另建 LP:
//!
//! 1. **翻转门槛**: 逐煤把价格往上 / 往下推, 在 5 元一档的网格上二分找"推多少元配方就
//!    换掉一成以上吨位". 门槛小于这种煤的煤价误差, 说明它的价格一估错配方就会变.
//!    这样的煤在平坦的最优解里往往一大片, 所以再按"占比 × 误差"(估错对成本的影响)
//!    排优先级, 只给前几种标"先问价".
//! 2. **更稳的配方**: 给每种煤加一笔与煤价误差成正比的风险溢价 λ·u 再解, 取多花
//!    不超过容差的最大 λ (λ ≤ 1). LP 天然专挑"估得偏低"的煤 (优化器诅咒), 误差越大的
//!    煤越容易被误选; 按误差加价正好抵消, 结果更多用价格有把握的煤.
//!
//! 重解一律去掉价格因子与本分析自身 (不再算配煤价值, 也不递归); 只改煤价不改约束,
//! 所以重解出的配方都满足合同. 价格平移加在出厂价上; 带采购水分折算的煤, 到厂成本的
//! 实际变动是平移量 × 折算系数 —— 估算价的煤没有采购条款, 不受影响.

use crate::model::*;
use crate::penalty::cif_eff_or_quoted;
use std::collections::{HashMap, HashSet};

/// 配方换掉多少吨位 (½Σ|Δx|) 算"变了".
const MOVED_SHARE: f64 = 0.10;
/// 翻转门槛的网格 (元/吨): 门槛总是它的整数倍, 等于它表示"不超过这么多".
pub(crate) const RESOLUTION: f64 = 5.0;
/// 占比低于此值的煤不算入选.
const MIN_SHARE: f64 = 0.005;
/// 入选的煤最多分析几种 (按占比取): 内点法遇到并列的煤会把份额摊开, 不设上限会重解太多次.
const MAX_SELECTED: usize = 8;
/// 没入选的煤最多分析几种 (按离入选最近的取).
const MAX_OUTSIDE: usize = 6;
/// 最多给几种煤标"先问价".
const MAX_ASK: usize = 3;
/// 风险溢价系数 λ 的二分次数.
const LAMBDA_STEPS: usize = 8;
/// 更稳的配方至少要把价格误差降低这么多 (元/吨) 才值得给出.
const MIN_ERROR_GAIN: f64 = 1.0;
/// 搜索范围与容差的上限, 防止一次请求触发过多重解.
const MAX_SWING_RANGE: f64 = 5000.0;
const MAX_TOLERANCE: f64 = 500.0;

/// `lp_request`: LP 实际用的请求 (估算价已并入); `original`: 原请求, 用来判断哪些煤是估算价;
/// `base`: `lp_request` 的求解结果 (须 ok); `solve`: 不带诊断的重解入口.
/// 返回报告与要挂到结果上的提示.
pub(crate) fn analyze(
    lp_request: &BlendRequest,
    original: &BlendRequest,
    base: &BlendResult,
    config: &PriceRobustness,
    solve: &dyn Fn(&BlendRequest) -> BlendResult,
) -> Result<(RobustnessReport, Vec<String>), String> {
    validate(config, original)?;
    let base_cost = base
        .cost
        .as_ref()
        .map(|cost| cost.net_per_ton)
        .ok_or("价格稳健性: 当前配方没有成本, 无法分析")?;

    let mut template = lp_request.clone();
    template.price_factor = None;
    template.price_robustness = None;
    template.fixed_ratios = None;

    let errors = price_errors(&template, original);
    let analysis = Analysis {
        template: &template,
        base_recipe: &base.recipe,
        solve,
        failures: std::cell::Cell::new(0),
    };
    let mut warnings = Vec::new();
    if base.blend_values.is_empty() {
        warnings.push("价格稳健性: 算不出配煤价值, 只分析了已入选的煤".into());
    }
    let swings = analysis.swings(base, &errors, config.swing_range);
    let price_error = weighted_error(&base.recipe, &errors);
    let robust = analysis.robust_recipe(&errors, base_cost, config.tolerance, price_error);
    if analysis.failures.get() > 0 {
        warnings.push(format!(
            "价格稳健性: {} 次改价重解失败, 部分门槛可能偏大",
            analysis.failures.get()
        ));
    }
    let report = RobustnessReport {
        tolerance: config.tolerance,
        swing_range: config.swing_range,
        price_error,
        swings,
        robust,
    };
    Ok((report, warnings))
}

fn validate(config: &PriceRobustness, request: &BlendRequest) -> Result<(), String> {
    let invalid = |what: String| Err(format!("价格稳健性配置无效 ({what}), 未做分析"));
    if !config.tolerance.is_finite() || !(0.0..=MAX_TOLERANCE).contains(&config.tolerance) {
        return invalid(format!("成本容差须在 0–{MAX_TOLERANCE} 元/吨"));
    }
    if !config.swing_range.is_finite()
        || !(RESOLUTION..=MAX_SWING_RANGE).contains(&config.swing_range)
    {
        return invalid(format!("搜索范围须在 {RESOLUTION}–{MAX_SWING_RANGE} 元/吨"));
    }
    let table = request
        .price_factor
        .as_ref()
        .and_then(|factor| factor.uncertainty.as_ref());
    if let Some(table) = table {
        let valid = |value: f64| value.is_finite() && value >= 0.0;
        if !valid(table.default)
            || !valid(table.quoted)
            || !table.by_coal_type.values().all(|v| valid(*v))
        {
            return invalid("煤价误差须为非负数".into());
        }
    }
    Ok(())
}

/// 煤名 → (煤价误差, 是否估算价). 估算价按煤种取, 有报价的取报价误差.
fn price_errors(template: &BlendRequest, original: &BlendRequest) -> HashMap<String, (f64, bool)> {
    let factor = original.price_factor.as_ref();
    let fallback = PriceUncertainty::default();
    let table = factor
        .and_then(|factor| factor.uncertainty.as_ref())
        .unwrap_or(&fallback);
    let estimated: HashSet<&str> = factor
        .filter(|factor| factor.estimate_unpriced)
        .map(|factor| {
            factor
                .extra_coals
                .iter()
                .filter(|coal| coal.fob <= 0.0)
                .map(|coal| coal.name.as_str())
                .collect()
        })
        .unwrap_or_default();
    template
        .coals
        .iter()
        .map(|coal| {
            let is_estimated = estimated.contains(coal.name.as_str());
            let error = if is_estimated {
                coal.coal_type
                    .as_ref()
                    .and_then(|kind| table.by_coal_type.get(kind))
                    .copied()
                    .unwrap_or(table.default)
            } else {
                table.quoted
            };
            (coal.name.clone(), (error, is_estimated))
        })
        .collect()
}

fn weighted_error(recipe: &HashMap<String, f64>, errors: &HashMap<String, (f64, bool)>) -> f64 {
    recipe
        .iter()
        .map(|(name, share)| share * errors.get(name).map_or(0.0, |(error, _)| *error))
        .sum()
}

/// 两个配方之间换掉的吨位比例 = ½Σ|Δx|.
fn moved_share(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let names: HashSet<&String> = a.keys().chain(b.keys()).collect();
    names
        .into_iter()
        .map(|name| (a.get(name).unwrap_or(&0.0) - b.get(name).unwrap_or(&0.0)).abs())
        .sum::<f64>()
        / 2.0
}

/// 门槛小于误差的煤 (价格一估错配方就变) 才有优先级: 占比 × 误差, 即估错对吨成本的
/// 影响. 没入选的煤按"入选时至少换掉一成"计.
fn ask_priority(swing: &PriceSwing) -> f64 {
    let fragile = [swing.rise, swing.fall]
        .into_iter()
        .flatten()
        .any(|threshold| threshold < swing.uncertainty);
    if fragile {
        swing.share.max(MOVED_SHARE) * swing.uncertainty
    } else {
        0.0
    }
}

struct Analysis<'a> {
    template: &'a BlendRequest,
    base_recipe: &'a HashMap<String, f64>,
    solve: &'a dyn Fn(&BlendRequest) -> BlendResult,
    /// 改价重解失败的次数 —— 只改价不该失败, 失败了要告诉用户门槛可能不准.
    failures: std::cell::Cell<usize>,
}

impl Analysis<'_> {
    fn share(&self, name: &str) -> f64 {
        self.base_recipe.get(name).copied().unwrap_or(0.0)
    }

    fn swings(
        &self,
        base: &BlendResult,
        errors: &HashMap<String, (f64, bool)>,
        range: f64,
    ) -> Vec<PriceSwing> {
        let mut swings: Vec<PriceSwing> = self
            .candidates(base, range)
            .into_iter()
            .map(|coal| {
                let share = self.share(&coal.name);
                let (uncertainty, estimated) =
                    errors.get(&coal.name).copied().unwrap_or((0.0, false));
                PriceSwing {
                    name: coal.name.clone(),
                    coal_type: coal.coal_type.clone(),
                    share,
                    estimated,
                    uncertainty,
                    // 没入选的煤涨价不会让它入选, 不必试.
                    rise: (share >= MIN_SHARE)
                        .then(|| self.threshold(&coal.name, 1.0, range))
                        .flatten(),
                    // 跌到 0 元以下没有意义.
                    fall: self.threshold(&coal.name, -1.0, range.min(coal.fob - 1.0)),
                    ask: false,
                }
            })
            .collect();
        let key = |swing: &PriceSwing| {
            swing
                .rise
                .unwrap_or(f64::INFINITY)
                .min(swing.fall.unwrap_or(f64::INFINITY))
        };
        swings.sort_by(|a, b| {
            ask_priority(b)
                .total_cmp(&ask_priority(a))
                .then(key(a).total_cmp(&key(b)))
                .then(b.share.total_cmp(&a.share))
                .then(a.name.cmp(&b.name))
        });
        for swing in swings.iter_mut().take(MAX_ASK) {
            swing.ask = ask_priority(swing) > 0.0;
        }
        swings
    }

    /// 入选的煤 (按占比取前几种), 加上离入选最近的几种 (到厂价 − 配煤价值 在搜索范围内).
    fn candidates(&self, base: &BlendResult, range: f64) -> Vec<&Coal> {
        let coals = &self.template.coals;
        let mut chosen: Vec<&Coal> = coals
            .iter()
            .filter(|coal| self.share(&coal.name) >= MIN_SHARE)
            .collect();
        chosen.sort_by(|a, b| self.share(&b.name).total_cmp(&self.share(&a.name)));
        chosen.truncate(MAX_SELECTED);
        let mut outside: Vec<(&Coal, f64)> = coals
            .iter()
            .filter(|coal| self.share(&coal.name) < MIN_SHARE)
            .filter_map(|coal| {
                let value = base.blend_values.get(&coal.name)?;
                let gap = cif_eff_or_quoted(coal) - value;
                (gap.is_finite() && gap <= range).then_some((coal, gap))
            })
            .collect();
        outside.sort_by(|a, b| a.1.total_cmp(&b.1));
        chosen.extend(outside.into_iter().take(MAX_OUTSIDE).map(|(coal, _)| coal));
        chosen
    }

    /// 单种煤价格往 `sign` 方向推多少元, 配方换掉 ≥ MOVED_SHARE 的吨位. 在 RESOLUTION
    /// 网格上二分, 返回最小的"已经变了"的格点; 范围内不变为 None.
    fn threshold(&self, name: &str, sign: f64, range: f64) -> Option<f64> {
        let steps = (range / RESOLUTION).floor();
        if steps < 1.0 {
            return None;
        }
        let changed = |step: f64| {
            let mut request = self.template.clone();
            if let Some(coal) = request.coals.iter_mut().find(|coal| coal.name == name) {
                coal.fob += sign * step * RESOLUTION;
            }
            let result = (self.solve)(&request);
            if !result.ok {
                self.failures.set(self.failures.get() + 1);
                return false;
            }
            moved_share(&result.recipe, self.base_recipe) >= MOVED_SHARE
        };
        if !changed(steps) {
            return None;
        }
        let (mut unchanged, mut moved) = (0.0, steps);
        while moved - unchanged > 1.0 {
            let middle = ((unchanged + moved) / 2.0).floor();
            if changed(middle) {
                moved = middle;
            } else {
                unchanged = middle;
            }
        }
        Some(moved * RESOLUTION)
    }

    /// 加风险溢价 λ·u 求解, 取多花不超过容差的最大 λ. 配方没变或误差没降就不给.
    fn robust_recipe(
        &self,
        errors: &HashMap<String, (f64, bool)>,
        base_cost: f64,
        tolerance: f64,
        base_error: f64,
    ) -> Option<RobustRecipe> {
        let attempt = |lambda: f64| -> Option<RobustRecipe> {
            let mut request = self.template.clone();
            for coal in &mut request.coals {
                coal.fob += lambda * errors.get(&coal.name).map_or(0.0, |(error, _)| *error);
            }
            let result = (self.solve)(&request);
            if !result.ok {
                self.failures.set(self.failures.get() + 1);
                return None;
            }
            let cost = self.cost_at_current_prices(&result.recipe)?;
            (cost <= base_cost + tolerance).then(|| RobustRecipe {
                price_error: weighted_error(&result.recipe, errors),
                // 平坦最优解上, 并列的另一个配方可能便宜 1e-7, 别显示成"多花 -0.0".
                extra_cost: (cost - base_cost).max(0.0),
                cost,
                recipe: result.recipe,
            })
        };
        let mut best = attempt(1.0);
        if best.is_none() {
            let (mut within, mut beyond) = (0.0, 1.0);
            for _ in 0..LAMBDA_STEPS {
                let middle = (within + beyond) / 2.0;
                match attempt(middle) {
                    Some(found) => {
                        within = middle;
                        best = Some(found);
                    }
                    None => beyond = middle,
                }
            }
        }
        best.filter(|robust| {
            moved_share(&robust.recipe, self.base_recipe) >= MIN_SHARE
                && robust.price_error <= base_error - MIN_ERROR_GAIN
        })
    }

    /// 按原价 (不加溢价) 验算配方的吨成本 —— 走验算模式, 与求解同一套成本算法.
    fn cost_at_current_prices(&self, recipe: &HashMap<String, f64>) -> Option<f64> {
        let mut request = self.template.clone();
        request.fixed_ratios = Some(recipe.clone());
        let result = (self.solve)(&request);
        result.cost.map(|cost| cost.net_per_ton)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs
            .iter()
            .map(|(name, share)| (name.to_string(), *share))
            .collect()
    }

    #[test]
    fn test_moved_share_counts_tonnage_once() {
        let a = recipe(&[("甲", 0.5), ("乙", 0.5)]);
        let b = recipe(&[("甲", 0.5), ("丙", 0.5)]);
        assert!((moved_share(&a, &b) - 0.5).abs() < 1e-12);
        assert!(moved_share(&a, &a).abs() < 1e-12);
    }

    #[test]
    fn test_weighted_error_ignores_unknown_coals() {
        let errors: HashMap<String, (f64, bool)> =
            [("甲".to_string(), (100.0, true))].into_iter().collect();
        let mix = recipe(&[("甲", 0.4), ("乙", 0.6)]);
        assert!((weighted_error(&mix, &errors) - 40.0).abs() < 1e-12);
    }
}
