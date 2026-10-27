//! 一铲最小量: 现场用铲车配煤, 一铲是最小单位. 每种煤要么不用, 要么至少一铲, 且按整铲配.
//!
//! 总铲数 N = round(采购量 / 一铲), 一铲占比 = 1/N —— 最小量与凑整用同一个口径.
//! 两步, 都只靠"改请求重解", 不另建 LP:
//!
//! 1. **最小量** (半连续变量): LP 解里不到一铲的煤, 要么去掉、要么提到一铲, 二选一.
//!    分支定界: 子节点在父节点上去掉这种煤, 或给它加"至少一铲"的配比下限
//!    (`min_ratios`) 再解. 先试去掉 (多数时候更便宜, 也最快找到一个可行解);
//!    节点成本不低于已知最好解就剪掉. 纯 LP 下这是精确的 (子节点只会更受限); 开了岩相修复
//!    或煤阶交互时重解带启发式迭代, 子节点成本不保证单调, 剪枝可能错过更便宜的组合.
//!    节点数有上限, 到了就用已找到的最好解.
//! 2. **整铲**: 各煤铲数按最大余数法凑成合计 N (入选的煤至少 1 铲), 再按铲数验算, 岩相
//!    按真实配比做精确复核. 凑整让某项超标时, 试着在两种煤之间挪一铲, 取不超标里最便宜的;
//!    都不行就退回未凑整的配方并提示.
//!
//! 配煤价值 (盈亏平衡价) 取原问题的 LP 解: 分支定界的节点去掉了煤、加了下限, 对偶价已变了味.
//! LP 解里占比 ≤ 1e-5 的煤不进 recipe, 凑整时按 0 处理.

use crate::model::*;
use crate::optimizer::{finalize_quality_status, review_petrography, FIXED_PETRO_NOTE};
use std::collections::{HashMap, HashSet};

/// 已有可行解后, 分支定界最多重解几次.
const MAX_NODES: usize = 40;
/// 还没找到可行解时放宽到这么多次 —— 内点法遇到并列的煤会把份额摊成一大片小份,
/// 要多去掉几轮才落地; 只用软上限会在找到任何解之前停下, 误报无解.
const MAX_NODES_UNTIL_FEASIBLE: usize = 200;
/// 总铲数超过这么多就不按铲配 (一铲吨数填错了, 如 0.001 吨).
const MAX_SHOVELS: f64 = 1_000_000.0;
/// 判"不到一铲"时容许的解算误差 (占比).
const RATIO_EPS: f64 = 1e-6;
/// 成本比较的容差 (元/吨).
const COST_EPS: f64 = 1e-9;

/// 重解入口: `(请求, 是否做不可行诊断)`.
pub(crate) type Resolve<'a> = dyn Fn(&BlendRequest, bool) -> BlendResult + 'a;

pub(crate) fn solve(request: &BlendRequest, resolve: &Resolve) -> BlendResult {
    let Some(tons) = request
        .shovel_tons
        .filter(|_| request.fixed_ratios.is_none())
    else {
        return resolve(request, true);
    };
    let skip = |note: String| {
        let mut result = resolve(request, true);
        result.warnings.push(note);
        result
    };
    let Some(quantity) = request.total_quantity else {
        return skip("没给采购总量, 一铲最小量不生效".into());
    };
    // 一铲吨数非正时输入校验会报错, 照常求解把它报出来.
    if !(tons.is_finite() && tons > 0.0) {
        return resolve(request, true);
    }
    if tons > quantity {
        return skip(format!(
            "一铲 {tons} 吨比采购总量 {quantity} 吨还大, 一铲最小量不生效"
        ));
    }
    let shovels = (quantity / tons).round();
    if shovels > MAX_SHOVELS {
        return skip(format!(
            "一铲 {tons} 吨太小 (超过 {MAX_SHOVELS} 铲), 一铲最小量不生效"
        ));
    }
    let total = shovels as u32;

    let root = resolve(request, true);
    // 无解则加了一铲只会更无解.
    if !root.ok {
        return root;
    }
    let mut search = Search {
        min_share: 1.0 / f64::from(total),
        resolve,
        nodes: 0,
        truncated: false,
        best: None,
    };
    search.explore(request.clone(), root.clone());
    let Some((node, lp)) = search.best else {
        let reason = if search.truncated {
            format!(
                "每种煤至少一铲 ({tons} 吨): 试了 {} 种组合仍没找到可行配方",
                search.nodes
            )
        } else {
            format!("每种煤至少一铲 ({tons} 吨) 时找不到可行配方")
        };
        return BlendResult::infeasible(&reason, root.warnings);
    };
    let mut result = whole_shovels(&node, lp, total, tons, resolve);
    result.blend_values = root.blend_values;
    if search.truncated {
        result
            .warnings
            .push("一铲最小量: 搜索到上限, 配方可能不是最便宜的".into());
    }
    result
}

struct Search<'a> {
    min_share: f64,
    resolve: &'a Resolve<'a>,
    nodes: usize,
    /// 有子节点因为节点数上限没解.
    truncated: bool,
    best: Option<(BlendRequest, BlendResult)>,
}

impl Search<'_> {
    fn explore(&mut self, node: BlendRequest, result: BlendResult) {
        let Some(cost) = net_cost(&result) else {
            return;
        };
        if self
            .best
            .as_ref()
            .and_then(|(_, best)| net_cost(best))
            .is_some_and(|best| cost >= best - COST_EPS)
        {
            return;
        }
        let Some(name) = self.short_of_one(&node, &result) else {
            self.best = Some((node, result));
            return;
        };
        // 用户自己给了下限的煤不能去掉, 只能往上提.
        let user_floored = node.min_ratios.get(&name).is_some_and(|floor| *floor > 0.0);
        let children = [
            (!user_floored).then(|| without(&node, &name)),
            Some(with_floor(&node, &name, self.min_share)),
        ];
        for child in children.into_iter().flatten() {
            let limit = if self.best.is_some() {
                MAX_NODES
            } else {
                MAX_NODES_UNTIL_FEASIBLE
            };
            if self.nodes >= limit {
                self.truncated = true;
                return;
            }
            self.nodes += 1;
            let child_result = (self.resolve)(&child, false);
            self.explore(child, child_result);
        }
    }

    /// 占比最小的、不到一铲的煤.
    fn short_of_one(&self, node: &BlendRequest, result: &BlendResult) -> Option<String> {
        result
            .recipe
            .iter()
            .filter(|(name, ratio)| {
                **ratio < self.min_share - RATIO_EPS
                    && node
                        .min_ratios
                        .get(*name)
                        .is_none_or(|floor| *floor < self.min_share)
            })
            .min_by(|left, right| left.1.total_cmp(right.1).then(left.0.cmp(right.0)))
            .map(|(name, _)| name.clone())
    }
}

fn net_cost(result: &BlendResult) -> Option<f64> {
    result.ok.then_some(result.cost.as_ref()?.net_per_ton)
}

fn without(node: &BlendRequest, name: &str) -> BlendRequest {
    let mut child = node.clone();
    child.coals.retain(|coal| coal.name != name);
    // 下限为 0 的键也要摘掉, 否则输入校验会报"不在煤池中".
    child.min_ratios.remove(name);
    child
}

fn with_floor(node: &BlendRequest, name: &str, min_share: f64) -> BlendRequest {
    let mut child = node.clone();
    let floor = child.min_ratios.entry(name.to_string()).or_insert(0.0);
    *floor = floor.max(min_share);
    child
}

/// 最大余数法: 按占比分 `total` 铲, 每种煤至少 1 铲. 调用方保证煤数 ≤ total.
fn apportion(ratios: &[f64], total: u32) -> Vec<u32> {
    let exact: Vec<f64> = ratios
        .iter()
        .map(|ratio| ratio * f64::from(total))
        .collect();
    let mut counts: Vec<u32> = exact
        .iter()
        .map(|value| (value.floor() as u32).max(1))
        .collect();
    loop {
        let assigned: u32 = counts.iter().sum();
        let surplus = |index: usize| exact[index] - f64::from(counts[index]);
        if assigned < total {
            let index = (0..counts.len())
                .max_by(|a, b| surplus(*a).total_cmp(&surplus(*b)))
                .expect("至少一种煤");
            counts[index] += 1;
        } else if assigned > total {
            let Some(index) = (0..counts.len())
                .filter(|index| counts[*index] > 1)
                .min_by(|a, b| surplus(*a).total_cmp(&surplus(*b)))
            else {
                return counts;
            };
            counts[index] -= 1;
        } else {
            return counts;
        }
    }
}

fn failed(result: &BlendResult) -> HashSet<&str> {
    result
        .indicator_check
        .iter()
        .filter(|check| check.status == EvaluationStatus::Fail)
        .map(|check| check.indicator.as_str())
        .collect()
}

/// 按铲数验算: 走验算路径出成本与体检, 再按真实配比做岩相精确复核 (验算路径只算线性代理,
/// 而铲数少时一铲就是一大截, 不能沿用凑整前的复核).
fn evaluate(
    node: &BlendRequest,
    names: &[&String],
    counts: &[u32],
    refine_iterations: usize,
    resolve: &Resolve,
) -> BlendResult {
    let shares: HashMap<String, f64> = names
        .iter()
        .zip(counts)
        .map(|(name, count)| ((*name).clone(), f64::from(*count)))
        .collect();
    let mut trial = node.clone();
    trial.shovel_tons = None;
    trial.min_ratios.clear();
    trial.fixed_ratios = Some(shares.clone());
    let mut result = resolve(&trial, false);
    if !result.ok {
        return result;
    }
    result
        .warnings
        .retain(|warning| warning != FIXED_PETRO_NOTE);
    let total = f64::from(counts.iter().sum::<u32>());
    let participating: Vec<(&Coal, f64)> = node
        .coals
        .iter()
        .filter_map(|coal| Some((coal, shares.get(&coal.name)? / total)))
        .collect();
    let specs: Vec<&Spec> = node.specs.iter().filter(|spec| spec.enabled).collect();
    let petro_spec = specs.iter().find(|spec| spec.indicator == "petro").copied();
    review_petrography(
        &mut result,
        &participating,
        petro_spec,
        node.truncate_decimal,
        refine_iterations,
    );
    finalize_quality_status(&mut result, &specs);
    for order in &mut result.orders {
        order.shovels = names
            .iter()
            .position(|name| **name == order.coal)
            .map(|index| counts[index]);
    }
    result
}

fn whole_shovels(
    node: &BlendRequest,
    lp: BlendResult,
    total: u32,
    tons: f64,
    resolve: &Resolve,
) -> BlendResult {
    let mut names: Vec<&String> = lp.recipe.keys().collect();
    names.sort();
    let ratios: Vec<f64> = names.iter().map(|name| lp.recipe[*name]).collect();
    let run = |counts: &[u32]| evaluate(node, &names, counts, lp.evaluation_iterations, resolve);
    let allowed = failed(&lp);
    let acceptable = |result: &BlendResult| result.ok && failed(result).is_subset(&allowed);

    let rounded = apportion(&ratios, total);
    let first = run(&rounded);
    let mut chosen = acceptable(&first).then_some(first);
    if chosen.is_none() {
        // 在两种煤之间挪一铲 (挪出的煤至少留 1 铲), 取不超标里最便宜的.
        for from in (0..rounded.len()).filter(|from| rounded[*from] > 1) {
            for to in (0..rounded.len()).filter(|to| *to != from) {
                let mut counts = rounded.clone();
                counts[from] -= 1;
                counts[to] += 1;
                let result = run(&counts);
                let cheaper = match (&chosen, net_cost(&result)) {
                    (_, None) => false,
                    (None, Some(_)) => true,
                    (Some(best), Some(cost)) => net_cost(best).is_some_and(|b| cost < b),
                };
                if acceptable(&result) && cheaper {
                    chosen = Some(result);
                }
            }
        }
    }
    chosen.unwrap_or_else(|| {
        let mut lp = lp;
        lp.warnings
            .push(format!("凑不成整铲 (一铲 {tons} 吨): 显示的是未凑整的配方"));
        lp
    })
}

#[cfg(test)]
mod tests {
    use super::apportion;

    #[test]
    fn test_apportion_largest_remainder() {
        assert_eq!(apportion(&[0.5, 0.3, 0.2], 10), vec![5, 3, 2]);
        assert_eq!(apportion(&[0.45, 0.35, 0.2], 3), vec![1, 1, 1]);
        assert_eq!(apportion(&[0.66, 0.34], 3), vec![2, 1]);
    }

    #[test]
    fn test_apportion_gives_every_coal_one() {
        // 0.04 × 20 = 0.8 铲, 也要给 1 铲, 从余数最小的煤里扣回来.
        let counts = apportion(&[0.04, 0.48, 0.48], 20);
        assert_eq!(counts.iter().sum::<u32>(), 20);
        assert!(counts.iter().all(|count| *count >= 1));
        assert_eq!(counts[0], 1);
    }
}
