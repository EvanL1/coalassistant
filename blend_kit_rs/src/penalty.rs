//! 买入侧扣款折算: 按单煤自身化验值与其采购合同条款修正到厂价.
//!
//! 与卖出侧的区别:
//!   - 买入侧扣款由该煤自身化验值决定, 是常数, 直接折进 LP 目标的成本系数;
//!   - 卖出侧扣款由混煤指标决定, 是决策变量的函数, 必须进 LP 作档位变量.
//!
//! 符号: 买入侧扣款让你**少付钱**, 因此是折扣(降成本), 与卖出侧方向相反.

use crate::model::*;

/// 凸分段线性扣款: 偏离量按档位依次计费, 便宜档先填满.
pub(crate) fn tiered_amount(tiers: &[PenaltyTier], deviation: f64) -> f64 {
    let mut remaining = deviation.max(0.0);
    let mut amount = 0.0;
    for tier in tiers {
        if remaining <= 0.0 {
            break;
        }
        let used = tier.width.map_or(remaining, |width| remaining.min(width));
        amount += used * tier.rate;
        remaining -= used;
    }
    amount
}

/// 按采购合同条款折算后的到厂价.
///
/// 返回 None 表示该煤自身化验值已越过其采购合同拒收线 —— 这种货实际收不进来, 应剔出煤池.
pub(crate) fn effective_cif(coal: &Coal) -> Option<f64> {
    let Some(terms) = &coal.purchase_terms else {
        return Some(coal.cif());
    };

    let mut deduction = 0.0;
    for clause in &terms.clauses {
        let Some(value) = coal.get(&clause.indicator) else {
            continue;
        };
        let (deviation, beyond_reject) = match clause.direction {
            Direction::Upper => (value - clause.guarantee, value > clause.penalty.reject),
            Direction::Lower => (clause.guarantee - value, value < clause.penalty.reject),
            Direction::Range => continue,
        };
        if beyond_reject {
            return None;
        }
        if deviation > 0.0 {
            deduction += tiered_amount(&clause.penalty.tiers, deviation);
        }
    }

    // 结算量 = 实收净重 × (1−实际水分)/(1−合同水分); 运费按实收湿重付, 不参与折算.
    let moisture_factor = match (terms.contract_moisture, coal.get("M")) {
        (Some(contract), Some(actual)) if contract < 100.0 => {
            let effective = match terms.moisture_excess_double_threshold {
                Some(threshold) if actual > threshold => 2.0 * actual - threshold,
                _ => actual,
            };
            (1.0 - effective / 100.0) / (1.0 - contract / 100.0)
        }
        _ => 1.0,
    };

    Some(coal.fob * moisture_factor + coal.frt - deduction)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(width: Option<f64>, rate: f64) -> PenaltyTier {
        PenaltyTier { width, rate }
    }

    #[test]
    fn test_tiered_amount_fills_cheap_tier_first() {
        let tiers = vec![tier(Some(1.0), 10.0), tier(None, 30.0)];
        assert!((tiered_amount(&tiers, 0.0) - 0.0).abs() < 1e-9);
        assert!((tiered_amount(&tiers, 0.5) - 5.0).abs() < 1e-9);
        assert!((tiered_amount(&tiers, 1.0) - 10.0).abs() < 1e-9);
        // 1.5 = 一档 1.0×10 + 二档 0.5×30
        assert!((tiered_amount(&tiers, 1.5) - 25.0).abs() < 1e-9);
    }

    fn coal_with(props: &[(&str, f64)], terms: Option<PurchaseTerms>) -> Coal {
        let mut map = std::collections::HashMap::new();
        for (key, value) in props {
            map.insert((*key).to_string(), *value);
        }
        Coal {
            name: "甲".into(),
            props: map,
            fob: 1000.0,
            frt: 100.0,
            petrography: None,
            purchase_terms: terms,
        }
    }

    fn ash_terms() -> PurchaseTerms {
        PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![tier(None, 80.0)],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        }
    }

    #[test]
    fn test_no_terms_keeps_quoted_price() {
        let coal = coal_with(&[("A", 10.0)], None);
        assert_eq!(effective_cif(&coal), Some(1100.0));
    }

    #[test]
    fn test_deduction_is_a_discount() {
        // 保证 10%, 实测 10.5%, 80 元/吨·% ⇒ 扣 40 元 ⇒ 成本下降
        let coal = coal_with(&[("A", 10.5)], Some(ash_terms()));
        assert!((effective_cif(&coal).unwrap() - 1060.0).abs() < 1e-9);
    }

    #[test]
    fn test_within_guarantee_has_no_deduction() {
        let coal = coal_with(&[("A", 9.2)], Some(ash_terms()));
        assert_eq!(effective_cif(&coal), Some(1100.0));
    }

    #[test]
    fn test_beyond_reject_returns_none() {
        let coal = coal_with(&[("A", 13.0)], Some(ash_terms()));
        assert_eq!(effective_cif(&coal), None, "越过采购合同拒收线的煤不应入池");
    }

    #[test]
    fn test_moisture_discounts_fob_but_not_freight() {
        // 结算量 = 净重×(1−10%)/(1−8%); 运费按实收湿重付, 不打折
        let terms = PurchaseTerms {
            clauses: Vec::new(),
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: None,
        };
        let coal = coal_with(&[("M", 10.0)], Some(terms));
        let expected = 1000.0 * (0.90 / 0.92) + 100.0;
        assert!(
            (effective_cif(&coal).unwrap() - expected).abs() < 1e-6,
            "应为 {expected}, 实得 {:?}",
            effective_cif(&coal)
        );
    }

    #[test]
    fn test_moisture_excess_double_threshold() {
        // 实测 14% 超阈值 12% ⇒ 有效水分 2×14−12 = 16%
        let terms = PurchaseTerms {
            clauses: Vec::new(),
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: Some(12.0),
        };
        let coal = coal_with(&[("M", 14.0)], Some(terms));
        let expected = 1000.0 * (0.84 / 0.92) + 100.0;
        assert!((effective_cif(&coal).unwrap() - expected).abs() < 1e-6);
    }
}
