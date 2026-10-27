//! 价格因子与求解器的衔接: 估价本身在 [`crate::pricing`], 这里只管
//! "无报价的煤按估算价并入煤池" 与 "求解后挂上煤价对比".

use std::collections::HashSet;

pub use crate::pricing::{evaluate, validate, PriceEvaluation, POINTS_BASE};

use crate::model::{BlendRequest, BlendResult, Coal};
use crate::quality::validate_coal;

/// extra_coals 的边界校验: 与煤池同一套单煤规则, 且煤名在 煤池 ∪ extra_coals 内唯一
/// (配煤价值按煤名对回, 重名会张冠李戴). 不带价格因子视为通过.
pub(crate) fn validate_extra_coals(request: &BlendRequest) -> Result<(), String> {
    let Some(factor) = &request.price_factor else {
        return Ok(());
    };
    let mut names: HashSet<&str> = request.coals.iter().map(|c| c.name.as_str()).collect();
    for coal in &factor.extra_coals {
        validate_coal(coal)?;
        if !names.insert(coal.name.as_str()) {
            return Err(format!("煤名重复: {}", coal.name));
        }
    }
    Ok(())
}

/// 估算价求解: `estimate_unpriced` 打开时, 把 extra_coals 按估算价 (fob = 应值) 并入煤池,
/// 返回改写后的请求和一条说明. `Ok(None)` = 不需要改写 (未开开关, 或配置无效 —— 后者由
/// [`attach`] 统一告警); `Err` = 要估却估不出 (没有市场价, 也没有可反推的报价).
///
/// 只改求解用的请求; 煤价对比仍按原请求评估, 所以这些煤在对比卡里照旧是"无报价".
pub(crate) fn with_estimated_prices(
    request: &BlendRequest,
) -> Result<Option<(BlendRequest, String)>, String> {
    let Some(factor) = request
        .price_factor
        .as_ref()
        .filter(|factor| factor.estimate_unpriced && !factor.extra_coals.is_empty())
    else {
        return Ok(None);
    };
    if validate_extra_coals(request).is_err() {
        return Ok(None);
    }
    let all: Vec<Coal> = request
        .coals
        .iter()
        .chain(&factor.extra_coals)
        .cloned()
        .collect();
    let Ok(eval) = evaluate(&all, factor) else {
        return Ok(None);
    };
    let Some(market) = eval.market else {
        return Err("无报价的煤要按估算价求解, 但没有市场价 (也没有可反推的报价), 估不出价".into());
    };
    let mut priced = request.clone();
    let mut count = 0;
    for coal in &factor.extra_coals {
        // 自带出厂价 (只是缺运费) 的照原价进; 没报价的才按估算价.
        if coal.fob > 0.0 {
            priced.coals.push(coal.clone());
            continue;
        }
        // 质量指数 ≤ 0 (极端劣质外推) 估不出有意义的价, 不进煤池.
        if let Some(value) = eval
            .coal_values
            .iter()
            .find(|value| value.name == coal.name && value.fair > 0.0)
        {
            priced.coals.push(Coal {
                fob: value.fair,
                ..coal.clone()
            });
            count += 1;
        }
    }
    if let Some(factor) = priced.price_factor.as_mut() {
        factor.extra_coals.clear();
    }
    let note = match factor.benchmarks.iter().map(|b| b.date.as_str()).max() {
        Some(latest) => format!(
            "{count} 种煤无报价, 按分煤种基准估价求解 ({} 条基准, 最新报价 {latest})",
            factor.benchmarks.len()
        ),
        None => format!("{count} 种煤无报价, 按估算价求解 (标准品 1000 点 = {market:.0} 元/吨)"),
    };
    Ok(Some((priced, note)))
}

/// 把评估结果挂到求解结果上 (有解无解都挂). 配置无效只加一条 warning, 不影响求解.
pub(crate) fn attach(request: &BlendRequest, result: &mut BlendResult) {
    let Some(factor) = &request.price_factor else {
        return;
    };
    if let Err(e) = validate_extra_coals(request) {
        result
            .warnings
            .push(format!("价格因子配置无效, 未做煤价对比: {e}"));
        return;
    }
    // 评估对象 = 进 LP 的煤 + 只评估的 extra_coals (按此顺序).
    let coals: Vec<Coal> = request
        .coals
        .iter()
        .chain(&factor.extra_coals)
        .cloned()
        .collect();
    match evaluate(&coals, factor) {
        Ok(eval) => {
            result.coal_values = eval.coal_values;
            for value in &mut result.coal_values {
                value.blend_value = result.blend_values.get(&value.name).copied();
            }
            result.market_price = eval.market;
            result.market_price_from_group = eval.market_from_group;
        }
        Err(e) => result
            .warnings
            .push(format!("价格因子配置无效, 未做煤价对比: {e}")),
    }
}
