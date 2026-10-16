//! 估价模块: 按质量指数和市场基准报价, 估每种煤值多少钱. 不依赖求解器, 只管价格.
//!
//! 对外入口: [`evaluate`] (Rust) 与 `estimate_json` (JSON in/out, 服务端 `/api/price`、
//! 命令行 `blend price` 用). 求解器 ([`crate::price_factor`]) 只是它的调用方之一:
//! 无报价的煤要按估算价进 LP 时来这里取价.
//!
//! 第一步 —— 质量指数 (每种煤 c, 每条启用的规则 i):
//!   - 取指标值 x; 煤缺该指标 → 跳过, 记入 missing.
//!   - 原始 x 落在 linear_range 外 → 照算, 记入 extrapolated.
//!   - 有 clamp 时先把 x 截到 [lo, hi].
//!   - Upper (越低越好): d = base − x; Lower (越高越好): d = x − base.
//!   - adj_i = d ≥ 0 ? premium_rate·d : discount_rate·d.
//!
//! 升贴水按**点**计: 标准品 = 1000 点, 质量指数 Q(c) = 1000 + Σ adj_i.
//!
//! 第二步 —— 定价, 按优先级:
//!   1. **基准报价** (`benchmarks`, 带日期的市场报价): 在同煤种的基准里取"升贴水差得最少"
//!      的一条 b (Σ|adj_i(c) − adj_i(b)|, 两边都有该指标的规则才算), 估价 = 基准价 × Q(c)/Q(b).
//!      同煤种没有基准就在全部基准里找. 这样肥煤对标肥煤、高硫对标高硫, 不必从标准品一路外推.
//!   2. **市场价** (`market_price`, 标准品 1000 点的价): 估价 = 市场价/1000 × Q(c).
//!   3. 都没有: 由有报价的煤反推市场价 = 1000 × mean(报价 / Q).
//!
//! 报价 (fob > 0) 只用于对比: Gap = 报价 − 估价, Ratio = 报价 / 估价.
//!
//! 为什么用点而不是元: 升贴水随行情按比例缩放 —— 市场价翻倍, 好煤坏煤的价差跟着翻倍,
//! 系数表不用改。1000 是贩煤人直觉里的整数基准 ("这煤 1050 点" = 比标准品好 5%)。

use std::collections::HashSet;

use crate::model::{
    Benchmark, Coal, CoalValue, Direction, PriceAdjust, PriceFactor, PricePart, PriceRequest,
    PriceResult, INDICATORS,
};
use crate::quality::validate_coal;

/// 一次价格因子评估的结果.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PriceEvaluation {
    pub coal_values: Vec<CoalValue>,
    /// 标准品 (1000 点) 对应的市场价.
    pub market: Option<f64>,
    pub market_from_group: bool,
}

/// 标准品的质量指数.
pub const POINTS_BASE: f64 = 1000.0;

fn validate_interval(
    indicator: &str,
    label: &str,
    interval: Option<[f64; 2]>,
) -> Result<(), String> {
    match interval {
        Some([lo, hi]) if !(lo.is_finite() && hi.is_finite() && lo < hi) => Err(format!(
            "{indicator} 的{label} [{lo}, {hi}] 无效, 须为有限数且下限 < 上限"
        )),
        _ => Ok(()),
    }
}

fn validate_adjust(rule: &PriceAdjust) -> Result<(), String> {
    let indicator = rule.indicator.as_str();
    if !INDICATORS.contains(&indicator) {
        return Err(format!("未知指标 {indicator}"));
    }
    if rule.direction == Direction::Range {
        return Err(format!(
            "{indicator} 的方向只能是 Upper 或 Lower, 不能是 Range"
        ));
    }
    if !rule.base.is_finite() {
        return Err(format!("{indicator} 的基准值必须是有限数"));
    }
    for (label, rate) in [
        ("升价费率", rule.premium_rate),
        ("扣价费率", rule.discount_rate),
    ] {
        if !(rate.is_finite() && rate >= 0.0) {
            return Err(format!(
                "{indicator} 的{label} {rate} 无效, 须为 ≥0 的有限数"
            ));
        }
    }
    validate_interval(indicator, "截断区间", rule.clamp)?;
    validate_interval(indicator, "线性区间", rule.linear_range)
}

/// 校验价格因子配置. 错误信息用于 warnings, 直接给用户看.
pub fn validate(factor: &PriceFactor) -> Result<(), String> {
    for (i, rule) in factor.adjusts.iter().enumerate() {
        validate_adjust(rule)?;
        if factor.adjusts[..i]
            .iter()
            .any(|r| r.indicator == rule.indicator)
        {
            return Err(format!("指标 {} 重复配置", rule.indicator));
        }
    }
    if let Some(p) = factor.market_price.filter(|p| !(p.is_finite() && *p > 0.0)) {
        return Err(format!("市场价 {p} 无效, 须为 >0 的有限数"));
    }
    validate_benchmarks(factor)
}

/// 基准报价: 名字唯一, 价为正, 指标是已知指标的有限数, 质量指数 > 0 (否则没法按比例换算).
fn validate_benchmarks(factor: &PriceFactor) -> Result<(), String> {
    let rules = enabled_rules(factor);
    let mut names = HashSet::new();
    for benchmark in &factor.benchmarks {
        let name = benchmark.name.trim();
        if name.is_empty() {
            return Err("基准报价缺名字".into());
        }
        if !names.insert(name) {
            return Err(format!("基准报价重名: {name}"));
        }
        let valid = |p: f64| p.is_finite() && p > 0.0;
        if !valid(benchmark.price) || !benchmark.current_price.is_none_or(valid) {
            return Err(format!("基准报价 {name} 的价格无效, 须为 >0 的有限数"));
        }
        if benchmark.date.trim().is_empty() {
            return Err(format!("基准报价 {name} 缺报价日"));
        }
        for (key, value) in &benchmark.props {
            if !INDICATORS.contains(&key.as_str()) || !value.is_finite() {
                return Err(format!("基准报价 {name} 的指标 {key}={value} 无效"));
            }
        }
        if adjust_coal(&probe(benchmark), &rules).points <= 0.0 {
            return Err(format!("基准报价 {name} 的质量指数 ≤ 0, 无法换算"));
        }
    }
    Ok(())
}

fn enabled_rules(factor: &PriceFactor) -> Vec<&PriceAdjust> {
    factor.adjusts.iter().filter(|rule| rule.enabled).collect()
}

/// 把基准品当成一种煤打分.
fn probe(benchmark: &Benchmark) -> Coal {
    Coal {
        name: benchmark.name.clone(),
        props: benchmark.props.clone(),
        fob: 0.0,
        frt: 0.0,
        petrography: None,
        purchase_terms: None,
        coal_type: benchmark.coal_type.clone(),
    }
}

/// 单条规则对一个指标值的升贴水 (点).
fn adjust_amount(rule: &PriceAdjust, raw: f64) -> f64 {
    let x = rule.clamp.map_or(raw, |[lo, hi]| raw.clamp(lo, hi));
    let d = match rule.direction {
        Direction::Upper => rule.base - x,
        _ => x - rule.base,
    };
    if d >= 0.0 {
        rule.premium_rate * d
    } else {
        rule.discount_rate * d
    }
}

/// 单种煤的升贴水明细; fair/gap/ratio 待市场价定下后再填. fob ≤ 0 视为没报价.
fn adjust_coal(coal: &Coal, rules: &[&PriceAdjust]) -> CoalValue {
    let mut value = CoalValue {
        name: coal.name.clone(),
        fob: (coal.fob > 0.0).then_some(coal.fob),
        adjustment: 0.0,
        points: POINTS_BASE,
        fair: 0.0,
        gap: None,
        ratio: None,
        blend_value: None,
        benchmark: None,
        parts: Vec::new(),
        missing: Vec::new(),
        extrapolated: Vec::new(),
    };
    for rule in rules {
        let Some(raw) = coal.get(&rule.indicator) else {
            value.missing.push(rule.indicator.clone());
            continue;
        };
        if rule
            .linear_range
            .is_some_and(|[lo, hi]| raw < lo || raw > hi)
        {
            value.extrapolated.push(rule.indicator.clone());
        }
        let amount = adjust_amount(rule, raw);
        value.adjustment += amount;
        value.points += amount;
        value.parts.push(PricePart {
            indicator: rule.indicator.clone(),
            amount,
        });
    }
    value
}

/// 对一组煤做价格评估. 配置非法时返回 Err (调用方转成 warning, 不影响求解).
pub fn evaluate(coals: &[Coal], factor: &PriceFactor) -> Result<PriceEvaluation, String> {
    validate(factor)?;
    let rules = enabled_rules(factor);
    let values: Vec<CoalValue> = coals.iter().map(|c| adjust_coal(c, &rules)).collect();
    if !factor.benchmarks.is_empty() {
        return Ok(price_by_benchmarks(coals, values, factor, &rules));
    }
    let (market, market_from_group) = match factor.market_price {
        Some(p) => (p, false),
        None => {
            // 只用有报价的煤反推; 质量指数 ≤ 0 的煤 (极端劣质外推) 没有有意义的"报价/指数".
            let ratios: Vec<f64> = values
                .iter()
                .filter(|v| v.points > 0.0)
                .filter_map(|v| v.fob.map(|fob| fob / v.points))
                .collect();
            if ratios.is_empty() {
                return Ok(PriceEvaluation::default());
            }
            (
                POINTS_BASE * ratios.iter().sum::<f64>() / ratios.len() as f64,
                true,
            )
        }
    };
    let coal_values = values
        .into_iter()
        .map(|mut v| {
            let fair = market / POINTS_BASE * v.points;
            v.fair = fair;
            v.gap = v.fob.map(|fob| fob - fair);
            v.ratio = v.fob.map(|fob| if fair > 0.0 { fob / fair } else { 0.0 });
            v
        })
        .collect();
    Ok(PriceEvaluation {
        coal_values,
        market: Some(market),
        market_from_group,
    })
}

/// 全部用 fixture, 不读 master. 系数是 master 默认表的点数版 (元系数按市场价 2000 折半).
/// 打过分的基准报价.
struct Anchor<'a> {
    benchmark: &'a Benchmark,
    value: CoalValue,
}

/// 两组升贴水差多少点: 只比两边都有指标的规则 —— 估价时就是靠这一段差额换算的.
fn distance(a: &CoalValue, b: &CoalValue) -> f64 {
    a.parts
        .iter()
        .filter_map(|part| {
            let other = b.parts.iter().find(|p| p.indicator == part.indicator)?;
            Some((part.amount - other.amount).abs())
        })
        .sum()
}

/// 同煤种里升贴水差得最少的基准; 同煤种没有就在全部基准里找. 并列取先列的.
fn nearest<'a>(coal: &Coal, value: &CoalValue, anchors: &'a [Anchor<'a>]) -> &'a Anchor<'a> {
    let same_type: Vec<&Anchor> = anchors
        .iter()
        .filter(|anchor| coal.coal_type.is_some() && anchor.benchmark.coal_type == coal.coal_type)
        .collect();
    let pool: Vec<&Anchor> = if same_type.is_empty() {
        anchors.iter().collect()
    } else {
        same_type
    };
    pool.into_iter()
        .min_by(|a, b| distance(value, &a.value).total_cmp(&distance(value, &b.value)))
        .expect("调用方保证至少一条基准")
}

/// 按基准报价定价: 估价 = 基准价 × Q(煤) / Q(基准). 标准品价取离标准品最近的基准反推.
fn price_by_benchmarks(
    coals: &[Coal],
    values: Vec<CoalValue>,
    factor: &PriceFactor,
    rules: &[&PriceAdjust],
) -> PriceEvaluation {
    let anchors: Vec<Anchor> = factor
        .benchmarks
        .iter()
        .map(|benchmark| Anchor {
            benchmark,
            value: adjust_coal(&probe(benchmark), rules),
        })
        .collect();
    let coal_values = coals
        .iter()
        .zip(values)
        .map(|(coal, mut value)| {
            let anchor = nearest(coal, &value, &anchors);
            let fair = anchor.benchmark.effective_price() * value.points / anchor.value.points;
            value.fair = fair;
            value.gap = value.fob.map(|fob| fob - fair);
            value.ratio = value
                .fob
                .map(|fob| if fair > 0.0 { fob / fair } else { 0.0 });
            value.benchmark = Some(anchor.benchmark.name.clone());
            value
        })
        .collect();
    let standard = anchors
        .iter()
        .min_by(|a, b| {
            a.value
                .adjustment
                .abs()
                .total_cmp(&b.value.adjustment.abs())
        })
        .map(|anchor| POINTS_BASE * anchor.benchmark.effective_price() / anchor.value.points);
    PriceEvaluation {
        coal_values,
        market: standard,
        market_from_group: false,
    }
}

/// 估价模块的入口: 校验煤 (与煤池同一套单煤规则, 煤名唯一) 后估价.
pub fn estimate(request: &PriceRequest) -> PriceResult {
    let failed = |reason: String| PriceResult {
        ok: false,
        reason: Some(reason),
        coal_values: Vec::new(),
        market_price: None,
    };
    let mut names = HashSet::new();
    for coal in &request.coals {
        if let Err(reason) = validate_coal(coal) {
            return failed(reason);
        }
        if !names.insert(coal.name.as_str()) {
            return failed(format!("煤名重复: {}", coal.name));
        }
    }
    match evaluate(&request.coals, &request.price_factor) {
        Ok(evaluation) => PriceResult {
            ok: true,
            reason: None,
            coal_values: evaluation.coal_values,
            market_price: evaluation.market,
        },
        Err(reason) => failed(reason),
    }
}

/// JSON in/out 入口: 输入 `{"coals": [...], "price_factor": {...}}`, 输出 [`PriceResult`].
pub fn estimate_json(input_json: &str) -> String {
    let result = match serde_json::from_str::<PriceRequest>(input_json) {
        Ok(request) => estimate(&request),
        Err(e) => PriceResult {
            ok: false,
            reason: Some(format!("JSON 解析失败: {e}")),
            coal_values: Vec::new(),
            market_price: None,
        },
    };
    serde_json::to_string(&result)
        .unwrap_or_else(|_| r#"{"ok":false,"reason":"序列化失败"}"#.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coal_from_tuple;

    const EPS: f64 = 1e-6;

    /// 顺序: (S, A, V, G, Y, petro, CSR, M, FOB, FRT). fob 为出厂价, frt 0.
    fn four_coals() -> Vec<Coal> {
        vec![
            coal_from_tuple(
                "荣欣",
                (0.4, 9.7, 24.0, 97.0, 23.0, 0.1, 75.0, 10.0, 2500.0, 0.0),
            ),
            coal_from_tuple(
                "新星",
                (1.6, 10.5, 20.0, 75.0, 17.0, 0.1, 65.0, 10.0, 2100.0, 0.0),
            ),
            coal_from_tuple(
                "北沟",
                (1.8, 8.6, 24.0, 94.0, 22.0, 0.1, 55.0, 10.0, 2200.0, 0.0),
            ),
            coal_from_tuple(
                "浮精",
                (0.8, 12.0, 24.0, 80.0, 20.0, 0.1, 58.0, 12.0, 2000.0, 0.0),
            ),
        ]
    }

    fn rule(
        indicator: &str,
        direction: Direction,
        base: f64,
        premium: f64,
        discount: f64,
    ) -> PriceAdjust {
        PriceAdjust {
            indicator: indicator.into(),
            direction,
            base,
            premium_rate: premium,
            discount_rate: discount,
            clamp: None,
            linear_range: None,
            enabled: true,
        }
    }

    /// 与 master 默认表同构的规则 (单位: 点/指标单位); csr 控制 CSR 规则是否启用.
    fn default_rules(csr: bool) -> PriceFactor {
        let mut s = rule("S", Direction::Upper, 1.30, 75.0, 125.0);
        s.clamp = Some([0.70, 100.0]);
        s.linear_range = Some([0.70, 1.60]);
        let mut a = rule("A", Direction::Upper, 10.5, 36.0, 36.0);
        a.linear_range = Some([8.0, 12.0]);
        let mut g = rule("G", Direction::Lower, 85.0, 2.5, 2.5);
        g.clamp = Some([40.0, 95.0]);
        let y = rule("Y", Direction::Lower, 15.0, 3.0, 3.0);
        let mut c = rule("CSR", Direction::Lower, 65.0, 5.0, 5.0);
        c.enabled = csr;
        PriceFactor {
            adjusts: vec![s, a, g, y, c],
            market_price: None,
            extra_coals: Vec::new(),
            estimate_unpriced: false,
            uncertainty: None,
            benchmarks: Vec::new(),
        }
    }

    fn with_market(csr: bool, market: f64) -> PriceFactor {
        PriceFactor {
            market_price: Some(market),
            ..default_rules(csr)
        }
    }

    fn value<'a>(eval: &'a PriceEvaluation, name: &str) -> &'a CoalValue {
        eval.coal_values
            .iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| panic!("没有 {name}"))
    }

    fn part(v: &CoalValue, indicator: &str) -> Option<f64> {
        v.parts
            .iter()
            .find(|p| p.indicator == indicator)
            .map(|p| p.amount)
    }

    fn close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() < EPS,
            "{what}: 实得 {actual}, 期望 {expected}"
        );
    }

    /// 手算质量指数 (CSR 启用, 标准品 1000 点):
    ///   荣欣 S0.4→截到0.70: +0.6×75=45; A +0.8×36=28.8; G97→95 +10×2.5=25; Y +8×3=24; CSR +10×5=50 ⇒ 1172.8
    ///   新星 S −0.3×125=−37.5; A 0; G −10×2.5=−25; Y +2×3=6; CSR 0 ⇒ 943.5
    ///   北沟 S −0.5×125=−62.5; A +1.9×36=68.4; G +22.5; Y +21; CSR −50 ⇒ 999.4
    ///   浮精 S +0.5×75=37.5; A −1.5×36=−54; G −12.5; Y +15; CSR −35 ⇒ 951
    #[test]
    fn test_points_are_1000_plus_adjustments() {
        let eval = evaluate(&four_coals(), &default_rules(true)).unwrap();
        for (name, points) in [
            ("荣欣", 1172.8),
            ("新星", 943.5),
            ("北沟", 999.4),
            ("浮精", 951.0),
        ] {
            let v = value(&eval, name);
            close(v.points, points, &format!("{name} 质量指数"));
            close(v.adjustment, points - 1000.0, &format!("{name} 升贴水点数"));
        }
        let names: Vec<_> = eval.coal_values.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["荣欣", "新星", "北沟", "浮精"]);
    }

    /// 应值 = 市场价/1000 × 质量指数. 市场价 2000 时与原来按元计的升贴水 (P0=2000) 完全一致:
    /// 荣欣 2000/1000 × 1172.8 = 2345.6, 新星 1887, 北沟 1998.8, 浮精 1902.
    #[test]
    fn test_fair_scales_points_by_market_price() {
        let eval = evaluate(&four_coals(), &with_market(true, 2000.0)).unwrap();
        assert!(!eval.market_from_group);
        close(eval.market.unwrap(), 2000.0, "市场价");
        for (name, fair) in [
            ("荣欣", 2345.6),
            ("新星", 1887.0),
            ("北沟", 1998.8),
            ("浮精", 1902.0),
        ] {
            let v = value(&eval, name);
            close(v.fair, fair, &format!("{name} 应值"));
            let fob = v.fob.unwrap();
            close(v.gap.unwrap(), fob - fair, &format!("{name} 差额"));
            close(v.ratio.unwrap(), fob / fair, &format!("{name} 比值"));
        }
    }

    /// 这个设计的意义: 行情翻倍, 好煤坏煤的价差跟着翻倍, 系数不用改.
    #[test]
    fn test_doubling_market_doubles_fair_values() {
        let at_2000 = evaluate(&four_coals(), &with_market(true, 2000.0)).unwrap();
        let at_4000 = evaluate(&four_coals(), &with_market(true, 4000.0)).unwrap();
        for (a, b) in at_2000.coal_values.iter().zip(&at_4000.coal_values) {
            close(b.fair, 2.0 * a.fair, &format!("{} 应值翻倍", a.name));
            close(
                b.points,
                a.points,
                &format!("{} 质量指数不随行情变", a.name),
            );
        }
    }

    /// 不填市场价: 市场价 = 1000 × mean(报价/质量指数), 于是各煤 报价/应值 的平均值恰为 1.
    #[test]
    fn test_market_inferred_from_group() {
        let eval = evaluate(&four_coals(), &default_rules(true)).unwrap();
        assert!(eval.market_from_group);
        let expected =
            1000.0 * (2500.0 / 1172.8 + 2100.0 / 943.5 + 2200.0 / 999.4 + 2000.0 / 951.0) / 4.0;
        close(eval.market.unwrap(), expected, "反推市场价");
        let mean_ratio: f64 = eval
            .coal_values
            .iter()
            .map(|v| v.ratio.unwrap())
            .sum::<f64>()
            / eval.coal_values.len() as f64;
        close(mean_ratio, 1.0, "平均 报价/应值");
    }

    #[test]
    fn test_extrapolation_flags() {
        let eval = evaluate(&four_coals(), &default_rules(true)).unwrap();
        // 北沟 S1.8 > 1.60 外推; 浮精 A=12 在区间边界上不算; 荣欣 S0.4 原始值 < 0.70 外推.
        assert!(value(&eval, "北沟").extrapolated.contains(&"S".to_string()));
        assert!(value(&eval, "浮精").extrapolated.is_empty());
        assert_eq!(value(&eval, "荣欣").extrapolated, vec!["S".to_string()]);
    }

    #[test]
    fn test_csr_disabled_has_no_csr_part() {
        let eval = evaluate(&four_coals(), &with_market(false, 2000.0)).unwrap();
        for v in &eval.coal_values {
            assert!(part(v, "CSR").is_none(), "{} 不应含 CSR 项", v.name);
            assert_eq!(v.parts.len(), 4);
        }
        // 关掉 CSR: 质量指数 1122.8 / 943.5 / 1049.4 / 986.
        close(value(&eval, "荣欣").points, 1122.8, "荣欣");
        close(value(&eval, "北沟").points, 1049.4, "北沟");
        close(value(&eval, "浮精").points, 986.0, "浮精");
    }

    fn single(s: f64) -> f64 {
        let factor = PriceFactor {
            adjusts: default_rules(false).adjusts.into_iter().take(1).collect(),
            market_price: Some(2000.0),
            extra_coals: Vec::new(),
            estimate_unpriced: false,
            uncertainty: None,
            benchmarks: Vec::new(),
        };
        let coal = coal_from_tuple(
            "甲",
            (s, 10.0, 24.0, 85.0, 15.0, 0.1, 65.0, 10.0, 2000.0, 0.0),
        );
        evaluate(&[coal], &factor).unwrap().coal_values[0].adjustment
    }

    #[test]
    fn test_sulfur_segment_boundaries() {
        close(single(0.70), 0.6 * 75.0, "S=0.70");
        close(single(0.50), 0.6 * 75.0, "S=0.50 截到 0.70");
        close(single(1.30), 0.0, "S=1.30 基准");
        close(single(1.60), -0.3 * 125.0, "S=1.60");
        close(single(1.10), 0.2 * 75.0, "S=1.10 升价");
    }

    #[test]
    fn test_g_clamped_to_95() {
        let factor = PriceFactor {
            adjusts: vec![default_rules(false).adjusts[2].clone()],
            market_price: Some(2000.0),
            extra_coals: Vec::new(),
            estimate_unpriced: false,
            uncertainty: None,
            benchmarks: Vec::new(),
        };
        let coal = coal_from_tuple(
            "甲",
            (1.0, 10.0, 24.0, 99.0, 15.0, 0.1, 65.0, 10.0, 2000.0, 0.0),
        );
        let v = &evaluate(&[coal], &factor).unwrap().coal_values[0];
        close(v.adjustment, 10.0 * 2.5, "G99 截到 95");
    }

    #[test]
    fn test_missing_indicator_is_skipped() {
        let mut coal = four_coals().remove(0);
        coal.props.remove("Y");
        let eval = evaluate(&[coal], &with_market(true, 2000.0)).unwrap();
        let v = &eval.coal_values[0];
        assert_eq!(v.missing, vec!["Y".to_string()]);
        assert!(part(v, "Y").is_none());
        close(v.points, 1172.8 - 24.0, "缺 Y 的质量指数");
    }

    /// 没报价 (fob ≤ 0) 的煤照样打分、给估算价, 只是没有差额; 市场价只用有报价的煤反推.
    #[test]
    fn test_unpriced_coal_scored_without_gap() {
        let mut coals = four_coals();
        coals[3].fob = 0.0;
        let eval = evaluate(&coals, &default_rules(true)).unwrap();
        assert_eq!(eval.coal_values.len(), 4);
        let market = 1000.0 * (2500.0 / 1172.8 + 2100.0 / 943.5 + 2200.0 / 999.4) / 3.0;
        close(eval.market.unwrap(), market, "只用前三种反推");
        let v = value(&eval, "浮精");
        assert_eq!((v.fob, v.gap, v.ratio), (None, None, None));
        close(v.points, 951.0, "浮精 质量指数");
        close(v.fair, market / 1000.0 * 951.0, "浮精 估算价");
    }

    /// 全都没报价但指定了市场价: 全部给估算价.
    #[test]
    fn test_no_priced_coal_with_market_scores_all() {
        let mut coals = four_coals();
        for c in &mut coals {
            c.fob = 0.0;
        }
        let eval = evaluate(&coals, &with_market(true, 2000.0)).unwrap();
        assert_eq!(eval.coal_values.len(), 4);
        close(value(&eval, "荣欣").fair, 2345.6, "荣欣 估算价");
        assert!(eval.coal_values.iter().all(|v| v.gap.is_none()));
    }

    #[test]
    fn test_no_priced_coal_gives_empty() {
        let mut coals = four_coals();
        for c in &mut coals {
            c.fob = 0.0;
        }
        let eval = evaluate(&coals, &default_rules(true)).unwrap();
        assert!(eval.coal_values.is_empty());
        assert!(eval.market.is_none());
    }

    #[test]
    fn test_validation_errors() {
        let bad = |edit: &dyn Fn(&mut PriceFactor)| {
            let mut f = default_rules(true);
            edit(&mut f);
            validate(&f).is_err()
        };
        assert!(validate(&default_rules(true)).is_ok());
        assert!(bad(&|f| f.adjusts[0].premium_rate = -1.0));
        assert!(bad(&|f| f.adjusts[0].discount_rate = f64::NAN));
        assert!(bad(&|f| f.adjusts[0].clamp = Some([1.0, 1.0])));
        assert!(bad(&|f| f.adjusts[0].linear_range = Some([2.0, 1.0])));
        assert!(bad(&|f| f.adjusts[1].indicator = "S".into()));
        assert!(bad(&|f| f.adjusts[0].direction = Direction::Range));
        assert!(bad(&|f| f.adjusts[0].indicator = "灰".into()));
        assert!(bad(&|f| f.adjusts[0].base = f64::INFINITY));
        assert!(bad(&|f| f.market_price = Some(f64::NAN)));
        assert!(bad(&|f| f.market_price = Some(0.0)), "市场价须 > 0");
        assert!(evaluate(
            &four_coals(),
            &PriceFactor {
                adjusts: vec![rule("X", Direction::Upper, 1.0, 1.0, 1.0)],
                market_price: None,
                extra_coals: Vec::new(),
                estimate_unpriced: false,
                uncertainty: None,
                benchmarks: Vec::new(),
            }
        )
        .is_err());
    }
}
