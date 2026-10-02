//! 价格因子: 按质量升贴水估每种煤"应值多少钱", 与报价 (出厂价) 对比.
//!
//! 只做评估, 绝不进 LP —— 求解拿到结果后统一挂 `coal_values`, 配方/成本/指标一概不变.
//!
//! 算法 (每种煤 c, 每条启用的规则 i):
//!   - 取指标值 x; 煤缺该指标 → 跳过, 记入 missing.
//!   - 原始 x 落在 linear_range 外 → 照算, 记入 extrapolated.
//!   - 有 clamp 时先把 x 截到 [lo, hi].
//!   - Upper (越低越好): d = base − x; Lower (越高越好): d = x − base.
//!   - adj_i = d ≥ 0 ? premium_rate·d : discount_rate·d.
//!
//! Adj(c) = Σ adj_i; 基准品价 P0 = 指定值, 或组内反推 mean(fob − Adj);
//! Fair = P0 + Adj, Gap = fob − Fair, Ratio = fob / Fair. fob ≤ 0 的煤不参与.
use crate::model::{
    BlendRequest, BlendResult, Coal, CoalValue, Direction, PriceAdjust, PriceFactor, PricePart,
    INDICATORS,
};

/// 一次价格因子评估的结果.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PriceEvaluation {
    pub coal_values: Vec<CoalValue>,
    pub base: Option<f64>,
    pub base_from_group: bool,
}

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
    match factor.base_price {
        Some(p) if !p.is_finite() => Err(format!("基准品价 {p} 不是有限数")),
        _ => Ok(()),
    }
}

/// 单条规则对一个指标值的升贴水 (元/吨).
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

/// 单种煤的升贴水明细; fair/gap/ratio 待 P0 定下后再填.
fn adjust_coal(coal: &Coal, rules: &[&PriceAdjust]) -> CoalValue {
    let mut value = CoalValue {
        name: coal.name.clone(),
        fob: coal.fob,
        adjustment: 0.0,
        fair: 0.0,
        gap: 0.0,
        ratio: 0.0,
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
    let rules: Vec<&PriceAdjust> = factor.adjusts.iter().filter(|r| r.enabled).collect();
    let values: Vec<CoalValue> = coals
        .iter()
        .filter(|c| c.fob > 0.0)
        .map(|c| adjust_coal(c, &rules))
        .collect();
    let (base, base_from_group) = match factor.base_price {
        Some(p) => (p, false),
        None if values.is_empty() => return Ok(PriceEvaluation::default()),
        None => {
            let total: f64 = values.iter().map(|v| v.fob - v.adjustment).sum();
            (total / values.len() as f64, true)
        }
    };
    let coal_values = values
        .into_iter()
        .map(|mut v| {
            v.fair = base + v.adjustment;
            v.gap = v.fob - v.fair;
            v.ratio = if v.fair > 0.0 { v.fob / v.fair } else { 0.0 };
            v
        })
        .collect();
    Ok(PriceEvaluation {
        coal_values,
        base: Some(base),
        base_from_group,
    })
}

/// 把评估结果挂到求解结果上 (有解无解都挂). 配置无效只加一条 warning, 不影响求解.
pub(crate) fn attach(request: &BlendRequest, result: &mut BlendResult) {
    let Some(factor) = &request.price_factor else {
        return;
    };
    match evaluate(&request.coals, factor) {
        Ok(eval) => {
            result.coal_values = eval.coal_values;
            result.price_base = eval.base;
            result.price_base_from_group = eval.base_from_group;
        }
        Err(e) => result
            .warnings
            .push(format!("价格因子配置无效, 未做煤价对比: {e}")),
    }
}

/// 全部用 fixture, 不读 master. 期望值由调研方案手算 (见 test_four_coals_csr_enabled).
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

    /// 与 master 默认表同构的规则; csr 控制 CSR 规则是否启用.
    fn default_rules(csr: bool) -> PriceFactor {
        let mut s = rule("S", Direction::Upper, 1.30, 150.0, 250.0);
        s.clamp = Some([0.70, 100.0]);
        s.linear_range = Some([0.70, 1.60]);
        let mut a = rule("A", Direction::Upper, 10.5, 72.0, 72.0);
        a.linear_range = Some([8.0, 12.0]);
        let mut g = rule("G", Direction::Lower, 85.0, 5.0, 5.0);
        g.clamp = Some([40.0, 95.0]);
        let y = rule("Y", Direction::Lower, 15.0, 6.0, 6.0);
        let mut c = rule("CSR", Direction::Lower, 65.0, 10.0, 10.0);
        c.enabled = csr;
        PriceFactor {
            adjusts: vec![s, a, g, y, c],
            base_price: None,
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

    /// 手算 (CSR 启用):
    ///   荣欣 S0.4→截到0.70: +0.6×150=90; A +0.8×72=57.6; G97→95 +10×5=50; Y +8×6=48; CSR +10×10=100 ⇒ +345.6
    ///   新星 S −0.3×250=−75; A 0; G −10×5=−50; Y +2×6=12; CSR 0 ⇒ −113
    ///   北沟 S −0.5×250=−125; A +1.9×72=136.8; G +45; Y +42; CSR −100 ⇒ −1.2
    ///   浮精 S +0.5×150=75; A −1.5×72=−108; G −25; Y +30; CSR −70 ⇒ −98
    ///   P0 = mean(2154.4, 2213, 2201.2, 2098) = 2166.65
    ///   Gap = fob − Adj − P0 = −12.25 / +46.35 / +34.55 / −68.65
    ///   (调研方案写的 −12.2/+46.3/+34.5/−68.7 是这组数四舍五入到一位小数.)
    #[test]
    fn test_four_coals_csr_enabled() {
        let eval = evaluate(&four_coals(), &default_rules(true)).unwrap();
        assert!(eval.base_from_group);
        close(eval.base.unwrap(), 2166.65, "P0");
        for (name, adj, gap) in [
            ("荣欣", 345.6, -12.25),
            ("新星", -113.0, 46.35),
            ("北沟", -1.2, 34.55),
            ("浮精", -98.0, -68.65),
        ] {
            let v = value(&eval, name);
            close(v.adjustment, adj, &format!("{name} Adj"));
            close(v.gap, gap, &format!("{name} Gap"));
            close(v.fair, 2166.65 + adj, &format!("{name} Fair"));
            close(v.ratio, v.fob / v.fair, &format!("{name} Ratio"));
        }
        // 结果按请求煤池顺序.
        let names: Vec<_> = eval.coal_values.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["荣欣", "新星", "北沟", "浮精"]);
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
        let eval = evaluate(&four_coals(), &default_rules(false)).unwrap();
        for v in &eval.coal_values {
            assert!(part(v, "CSR").is_none(), "{} 不应含 CSR 项", v.name);
            assert_eq!(v.parts.len(), 4);
        }
        // 关掉 CSR: Adj = 245.6 / −113 / 98.8 / −28, P0 = 2149.15.
        close(eval.base.unwrap(), 2149.15, "P0");
        close(value(&eval, "荣欣").gap, 105.25, "荣欣 Gap");
        close(value(&eval, "新星").gap, 63.85, "新星 Gap");
        close(value(&eval, "北沟").gap, -47.95, "北沟 Gap");
        close(value(&eval, "浮精").gap, -121.15, "浮精 Gap");
    }

    fn single(s: f64) -> f64 {
        let factor = PriceFactor {
            adjusts: default_rules(false).adjusts.into_iter().take(1).collect(),
            base_price: Some(2000.0),
        };
        let coal = coal_from_tuple(
            "甲",
            (s, 10.0, 24.0, 85.0, 15.0, 0.1, 65.0, 10.0, 2000.0, 0.0),
        );
        evaluate(&[coal], &factor).unwrap().coal_values[0].adjustment
    }

    #[test]
    fn test_sulfur_segment_boundaries() {
        close(single(0.70), 0.6 * 150.0, "S=0.70");
        close(single(0.50), 0.6 * 150.0, "S=0.50 截到 0.70");
        close(single(1.30), 0.0, "S=1.30 基准");
        close(single(1.60), -0.3 * 250.0, "S=1.60");
        close(single(1.10), 0.2 * 150.0, "S=1.10 升价");
    }

    #[test]
    fn test_g_clamped_to_95() {
        let factor = PriceFactor {
            adjusts: vec![default_rules(false).adjusts[2].clone()],
            base_price: Some(2000.0),
        };
        let coal = coal_from_tuple(
            "甲",
            (1.0, 10.0, 24.0, 99.0, 15.0, 0.1, 65.0, 10.0, 2000.0, 0.0),
        );
        let v = &evaluate(&[coal], &factor).unwrap().coal_values[0];
        close(v.adjustment, 10.0 * 5.0, "G99 截到 95");
    }

    #[test]
    fn test_missing_indicator_is_skipped() {
        let mut coal = four_coals().remove(0);
        coal.props.remove("Y");
        let eval = evaluate(
            &[coal],
            &PriceFactor {
                base_price: Some(2000.0),
                ..default_rules(true)
            },
        )
        .unwrap();
        let v = &eval.coal_values[0];
        assert_eq!(v.missing, vec!["Y".to_string()]);
        assert!(part(v, "Y").is_none());
        close(v.adjustment, 345.6 - 48.0, "缺 Y 的 Adj");
    }

    #[test]
    fn test_given_base_price_is_used() {
        let factor = PriceFactor {
            base_price: Some(2000.0),
            ..default_rules(true)
        };
        let eval = evaluate(&four_coals(), &factor).unwrap();
        assert!(!eval.base_from_group);
        close(eval.base.unwrap(), 2000.0, "P0");
        close(value(&eval, "荣欣").fair, 2345.6, "荣欣 Fair");
        close(value(&eval, "荣欣").gap, 154.4, "荣欣 Gap");
    }

    #[test]
    fn test_group_base_gaps_sum_to_zero() {
        let eval = evaluate(&four_coals(), &default_rules(true)).unwrap();
        let total: f64 = eval.coal_values.iter().map(|v| v.gap).sum();
        assert!(total.abs() < 1e-6, "Σgap = {total}");
    }

    #[test]
    fn test_non_positive_fob_excluded() {
        let mut coals = four_coals();
        coals[3].fob = 0.0;
        let eval = evaluate(&coals, &default_rules(true)).unwrap();
        assert_eq!(eval.coal_values.len(), 3);
        // P0 只用前三种: mean(2154.4, 2213, 2201.2).
        close(eval.base.unwrap(), (2154.4 + 2213.0 + 2201.2) / 3.0, "P0");
    }

    #[test]
    fn test_no_priced_coal_gives_empty() {
        let mut coals = four_coals();
        for c in &mut coals {
            c.fob = 0.0;
        }
        let eval = evaluate(&coals, &default_rules(true)).unwrap();
        assert!(eval.coal_values.is_empty());
        assert!(eval.base.is_none());
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
        assert!(bad(&|f| f.base_price = Some(f64::NAN)));
        assert!(evaluate(
            &four_coals(),
            &PriceFactor {
                adjusts: vec![rule("X", Direction::Upper, 1.0, 1.0, 1.0)],
                base_price: None,
            }
        )
        .is_err());
    }
}
