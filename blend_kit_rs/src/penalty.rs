//! 买入侧扣款折算: 按单煤自身化验值与其采购合同条款修正到厂价.
//!
//! 与卖出侧的区别:
//!   - 买入侧扣款由该煤自身化验值决定, 是常数, 直接折进 LP 目标的成本系数;
//!   - 卖出侧扣款由混煤指标决定, 是决策变量的函数, 必须进 LP 作档位变量.
//!
//! 符号: 买入侧扣款让你**少付钱**, 因此是折扣(降成本), 与卖出侧方向相反.

use crate::model::*;

/// 某条采购条款把这煤挡在门外的原因.
///
/// 带上指标与两个数字, 是为了让剔煤告警能指名道姓 —— 与同一处的"缺指标"分支对齐,
/// 后者也会报出具体是哪几项缺了.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CullReason {
    pub indicator: String,
    /// 该煤这项的实测值.
    pub value: f64,
    /// 其采购合同在这项上的拒收线.
    pub reject: f64,
}

/// 凸分段线性扣款: 偏离量按档位依次计费, 便宜档先填满.
///
/// 卖出侧对同一份档位表走的是 LP 档位列(见 `optimizer::append_priced_blocks`),
/// 两条实现必须对同样的 tiers 与偏离量算出同样的钱; 凸性(rate 严格递增)是它们
/// 能一致的前提, 由 `quality::validate_penalty` 统一保证.
fn tiered_amount(tiers: &[PenaltyTier], deviation: f64) -> f64 {
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

/// 采购条款里该煤缺化验值的指标 —— 这些条款算不出扣款, 被 [`cif_eff`] 跳过.
///
/// 只告警不剔煤是刻意的: 全局模板把同一套条款套到每个煤上, 而化验单缺项是常态,
/// 缺一项就否掉一个煤会在模板下连带废掉大批本可正常采购的货.
pub(crate) fn clauses_missing_assay(coal: &Coal) -> Vec<&str> {
    let Some(terms) = &coal.purchase_terms else {
        return Vec::new();
    };
    terms
        .clauses
        .iter()
        .filter(|clause| coal.get(&clause.indicator).is_none())
        .map(|clause| clause.indicator.as_str())
        .collect()
}

/// 按采购合同条款折算后的到厂价.
///
/// 返回 `Err` 表示该煤自身化验值已越过其采购合同拒收线 —— 这种货实际收不进来,
/// 应剔出煤池; `CullReason` 说明是哪条条款、差在哪。
pub(crate) fn cif_eff(coal: &Coal) -> Result<f64, CullReason> {
    let Some(terms) = &coal.purchase_terms else {
        return Ok(coal.cif());
    };

    let mut deduction = 0.0;
    for clause in &terms.clauses {
        // 缺化验值的条款算不出扣款, 跳过; 调用方用 clauses_missing_assay 单独告警.
        let Some(value) = coal.get(&clause.indicator) else {
            continue;
        };
        let (deviation, beyond_reject) = match clause.direction {
            Direction::Upper => (value - clause.guarantee, value > clause.penalty.reject),
            Direction::Lower => (clause.guarantee - value, value < clause.penalty.reject),
            // Range 已被 quality::validate_purchase_terms 的 Direction::Range 分支拒在门外,
            // 这里走不到. 跳过而非 panic: 两个 crate 的 release profile 都设了
            // panic = "abort", 真被走到会直接崩掉整个 server 进程而非只失败一次请求.
            Direction::Range => continue,
        };
        if beyond_reject {
            return Err(CullReason {
                indicator: clause.indicator.clone(),
                value,
                reject: clause.penalty.reject,
            });
        }
        if deviation > 0.0 {
            deduction += tiered_amount(&clause.penalty.tiers, deviation);
        }
    }

    // 扣量机制: 结算量 = 实收净重 × (1−M_eff/100)/(1−M合/100).
    // 你收到 W 吨湿煤, 却只按折算后的吨数付货款; 而运费按实收湿重全额付, 不参与折算
    // —— 所以湿煤不是中性的, 它是净亏一笔运费.
    //
    // M_eff 是计价用的"有效水分": 超过双倍阈值的部分按 2 倍计入, 即
    //   2·M − 阈值  ≡  阈值 + 2·(M − 阈值)
    // 左式是算式, 右式是合同原话("超出部分双倍"), 两者恒等.
    //
    // 水分走扣量时不会再有 M 条款走扣价: 两种机制互斥, 由 validate_purchase_terms 保证.
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

    Ok(coal.fob * moisture_factor + coal.frt - deduction)
}

/// 求解期用的到厂价: 越拒收线的煤在候选筛选阶段已被剔除, 这里兜底回退到报价原值.
pub(crate) fn cif_eff_or_quoted(coal: &Coal) -> f64 {
    cif_eff(coal).unwrap_or_else(|_| coal.cif())
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
        assert_eq!(cif_eff(&coal), Ok(1100.0));
    }

    #[test]
    fn test_deduction_is_a_discount() {
        // 保证 10%, 实测 10.5%, 80 元/吨·% ⇒ 扣 40 元 ⇒ 成本下降
        let coal = coal_with(&[("A", 10.5)], Some(ash_terms()));
        assert!((cif_eff(&coal).unwrap() - 1060.0).abs() < 1e-9);
    }

    #[test]
    fn test_within_guarantee_has_no_deduction() {
        let coal = coal_with(&[("A", 9.2)], Some(ash_terms()));
        assert_eq!(cif_eff(&coal), Ok(1100.0));
    }

    /// 越拒收线时要说清是哪条条款越的线 —— 告警靠这三个字段指名道姓.
    #[test]
    fn test_beyond_reject_reports_the_offending_clause() {
        let coal = coal_with(&[("A", 13.0)], Some(ash_terms()));
        assert_eq!(
            cif_eff(&coal),
            Err(CullReason {
                indicator: "A".into(),
                value: 13.0,
                reject: 12.0,
            }),
            "越过采购合同拒收线的煤不应入池, 且要报出指标/实测值/拒收线"
        );
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
            (cif_eff(&coal).unwrap() - expected).abs() < 1e-6,
            "应为 {expected}, 实得 {:?}",
            cif_eff(&coal)
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
        assert!((cif_eff(&coal).unwrap() - expected).abs() < 1e-6);
    }

    /// 缺化验值的条款只是跳过, 煤仍按报价可用 —— 但缺了哪几项必须能报出来.
    #[test]
    fn test_clause_missing_assay_is_skipped_and_reported() {
        let terms = PurchaseTerms {
            clauses: vec![
                PurchaseClause {
                    indicator: "A".into(),
                    direction: Direction::Upper,
                    guarantee: 10.0,
                    penalty: Penalty {
                        tiers: vec![tier(None, 80.0)],
                        reject: 12.0,
                    },
                },
                PurchaseClause {
                    indicator: "S".into(),
                    direction: Direction::Upper,
                    guarantee: 1.0,
                    penalty: Penalty {
                        tiers: vec![tier(None, 50.0)],
                        reject: 2.0,
                    },
                },
            ],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        // 只有灰分有化验值, 硫缺失.
        let coal = coal_with(&[("A", 10.5)], Some(terms));
        assert_eq!(
            clauses_missing_assay(&coal),
            vec!["S"],
            "缺化验值的条款应被点名"
        );
        assert!(
            (cif_eff(&coal).unwrap() - 1060.0).abs() < 1e-9,
            "缺值条款跳过, 有值条款照扣"
        );
    }

    #[test]
    fn test_no_terms_has_no_missing_clauses() {
        let coal = coal_with(&[("A", 10.0)], None);
        assert!(clauses_missing_assay(&coal).is_empty());
    }

    #[test]
    fn test_cif_eff_or_quoted_falls_back_on_cull() {
        let coal = coal_with(&[("A", 13.0)], Some(ash_terms()));
        assert_eq!(cif_eff_or_quoted(&coal), 1100.0, "越线时兜底回退报价原值");
    }
}
