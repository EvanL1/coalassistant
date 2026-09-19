//! 指标的业务判定和 LP 可表达式.
//!
//! 核心分离:
//! - `AcceptanceRule`: 合同如何判定报告值；
//! - `margin`: 模型不确定性导致的内部收紧；
//! - `MetricFormula`: LP 代理和最终评估共用的表达式。

use crate::model::*;

// 与 Clarabel 的 1e-8 可行性精度对齐。判定档位的开放边界另按小数位动态留缝。
const CHECK_TOLERANCE: f64 = 1e-8;
const BINDING_TOL: f64 = 0.05;

#[derive(Debug, Clone)]
pub(crate) struct MetricFormula {
    /// 兼容展示的简单加权代理系数.
    pub proxy_coefficients: Vec<f64>,
    /// 最终评估的分子系数.
    pub numerators: Vec<f64>,
    /// 最终评估的分母系数.
    pub denominators: Vec<f64>,
    pub intercept: f64,
    pub method: EvaluationMethod,
    pub verified: bool,
    pub uncertainty: Option<f64>,
    pub model: Option<ModelSummary>,
}

impl MetricFormula {
    pub fn linear(coefficients: Vec<f64>, method: EvaluationMethod, verified: bool) -> Self {
        let n = coefficients.len();
        Self {
            proxy_coefficients: coefficients.clone(),
            numerators: coefficients,
            denominators: vec![1.0; n],
            intercept: 0.0,
            method,
            verified,
            uncertainty: None,
            model: None,
        }
    }

    pub fn proxy_value(&self, ratios: &[f64]) -> f64 {
        dot(&self.proxy_coefficients, ratios)
    }

    pub fn evaluate(&self, ratios: &[f64]) -> Option<f64> {
        let denominator = dot(&self.denominators, ratios);
        if !denominator.is_finite() || denominator <= 1e-12 {
            return None;
        }
        Some(self.intercept + dot(&self.numerators, ratios) / denominator)
    }

    /// 把 `evaluate(x) <= upper` 编译为 `a·x <= b`.
    pub fn upper_constraint(&self, upper: f64) -> (Vec<f64>, f64) {
        let shifted = upper - self.intercept;
        (
            self.numerators
                .iter()
                .zip(&self.denominators)
                .map(|(numerator, denominator)| numerator - shifted * denominator)
                .collect(),
            0.0,
        )
    }

    /// 把 `evaluate(x) >= lower` 编译为 `a·x <= b`.
    pub fn lower_constraint(&self, lower: f64) -> (Vec<f64>, f64) {
        let shifted = lower - self.intercept;
        (
            self.numerators
                .iter()
                .zip(&self.denominators)
                .map(|(numerator, denominator)| shifted * denominator - numerator)
                .collect(),
            0.0,
        )
    }
}

fn dot(values: &[f64], ratios: &[f64]) -> f64 {
    values
        .iter()
        .zip(ratios)
        .map(|(value, ratio)| value * ratio)
        .sum()
}

pub(crate) fn acceptance_rule(spec: &Spec, legacy_truncate: bool) -> AcceptanceRule {
    spec.acceptance.clone().unwrap_or_else(|| {
        if legacy_truncate && spec.indicator != "petro" {
            AcceptanceRule {
                mode: AcceptanceMode::Truncate,
                decimals: Some(1),
                tolerance: 0.0,
            }
        } else {
            AcceptanceRule::default()
        }
    })
}

fn scale(rule: &AcceptanceRule) -> f64 {
    10_f64.powi(i32::from(rule.decimals.unwrap_or(0).min(6)))
}

fn strict_bound_epsilon(rule: &AcceptanceRule) -> f64 {
    let quantum = 1.0 / scale(rule);
    (quantum * 1e-4).max(CHECK_TOLERANCE * 10.0)
}

pub(crate) fn judged_value(value: f64, rule: &AcceptanceRule) -> f64 {
    let scale = scale(rule);
    let scaled = value * scale;
    let stabilized = match rule.mode {
        AcceptanceMode::Raw => scaled,
        AcceptanceMode::Truncate => {
            let boundary = scaled.round();
            if (scaled - boundary).abs() <= CHECK_TOLERANCE * scale {
                boundary
            } else {
                scaled
            }
        }
        AcceptanceMode::Round => {
            let boundary = (scaled - 0.5).round() + 0.5;
            if (scaled - boundary).abs() <= CHECK_TOLERANCE * scale {
                boundary
            } else {
                scaled
            }
        }
    };
    match rule.mode {
        AcceptanceMode::Raw => value,
        AcceptanceMode::Truncate => stabilized.trunc() / scale,
        AcceptanceMode::Round => stabilized.round() / scale,
    }
}

pub(crate) fn effective_upper(bound: f64, rule: &AcceptanceRule) -> f64 {
    let accepted = bound + rule.tolerance.max(0.0);
    match rule.mode {
        AcceptanceMode::Raw => accepted,
        AcceptanceMode::Truncate => {
            ((accepted * scale(rule)).floor() + 1.0) / scale(rule) - strict_bound_epsilon(rule)
        }
        AcceptanceMode::Round => {
            ((accepted * scale(rule)).floor() + 0.5) / scale(rule) - strict_bound_epsilon(rule)
        }
    }
}

pub(crate) fn effective_lower(bound: f64, rule: &AcceptanceRule) -> f64 {
    let accepted = bound - rule.tolerance.max(0.0);
    match rule.mode {
        AcceptanceMode::Raw => accepted,
        AcceptanceMode::Truncate => (accepted * scale(rule)).ceil() / scale(rule),
        AcceptanceMode::Round => ((accepted * scale(rule)).ceil() - 0.5) / scale(rule),
    }
}

pub(crate) struct CheckOutcome {
    pub judged: f64,
    pub slack: Option<f64>,
    pub binding: bool,
    pub status: EvaluationStatus,
}

pub(crate) fn check_value(
    value: f64,
    spec: Option<&Spec>,
    legacy_truncate: bool,
    verified: bool,
) -> CheckOutcome {
    let Some(spec) = spec else {
        return CheckOutcome {
            judged: value,
            slack: None,
            binding: false,
            status: if verified {
                EvaluationStatus::Pass
            } else {
                EvaluationStatus::Unverified
            },
        };
    };
    let rule = acceptance_rule(spec, legacy_truncate);
    let judged = judged_value(value, &rule);
    let use_max = matches!(spec.direction, Direction::Upper | Direction::Range);
    let use_min = matches!(spec.direction, Direction::Lower | Direction::Range);
    let mut accepted_slacks = Vec::new();
    let mut raw_pass = true;
    let mut judged_pass = true;

    if use_max {
        if let Some(maximum) = spec.max {
            accepted_slacks.push(effective_upper(maximum, &rule) - value);
            raw_pass &= value <= maximum + CHECK_TOLERANCE;
            judged_pass &= judged <= maximum + rule.tolerance.max(0.0) + CHECK_TOLERANCE;
        }
    }
    if use_min {
        if let Some(minimum) = spec.min {
            accepted_slacks.push(value - effective_lower(minimum, &rule));
            raw_pass &= value + CHECK_TOLERANCE >= minimum;
            judged_pass &= judged + CHECK_TOLERANCE >= minimum - rule.tolerance.max(0.0);
        }
    }

    let slack = accepted_slacks.into_iter().reduce(f64::min);
    let binding =
        slack.is_some_and(|distance| distance > -CHECK_TOLERANCE && distance < BINDING_TOL);
    let accepted = slack.is_none_or(|distance| distance >= -CHECK_TOLERANCE) && judged_pass;
    let status = if !accepted {
        EvaluationStatus::Fail
    } else if !verified {
        EvaluationStatus::Unverified
    } else if !raw_pass {
        EvaluationStatus::TolerancePass
    } else {
        EvaluationStatus::Pass
    };

    CheckOutcome {
        judged,
        slack,
        binding,
        status,
    }
}

pub(crate) fn formula_for(indicator: &str, coals: &[&Coal]) -> Option<MetricFormula> {
    let proxy: Vec<f64> = coals
        .iter()
        .map(|coal| coal.get(indicator))
        .collect::<Option<Vec<_>>>()?;

    let (method, verified) = match indicator {
        // 这三项在求解器中按质量加权，算法本身可精确复算。
        "S" | "A" | "M" => (EvaluationMethod::Linear, true),
        // 其余指标保留线性代理，但不把代理冒充已验证的非线性结果。
        "V" | "G" | "Y" | "petro" | "CSR" => (EvaluationMethod::ProvisionalLinear, false),
        _ => (EvaluationMethod::Unavailable, false),
    };
    Some(MetricFormula::linear(proxy, method, verified))
}

pub(crate) fn validate_request(request: &BlendRequest) -> Result<(), String> {
    if request.coals.is_empty() {
        return Err("煤池不能为空".into());
    }
    if request
        .total_quantity
        .is_some_and(|quantity| !quantity.is_finite() || quantity <= 0.0)
    {
        return Err("采购总量必须是正数".into());
    }

    let mut names = std::collections::HashSet::new();
    for coal in &request.coals {
        if coal.name.trim().is_empty() {
            return Err("煤名不能为空".into());
        }
        if !names.insert(coal.name.as_str()) {
            return Err(format!("煤名重复: {}", coal.name));
        }
        if !coal.fob.is_finite() || !coal.frt.is_finite() || coal.fob < 0.0 || coal.frt < 0.0 {
            return Err(format!("{} 的价格必须是非负有限数", coal.name));
        }
        if coal.props.iter().any(|(indicator, value)| {
            !INDICATORS.contains(&indicator.as_str()) || !value.is_finite() || *value < 0.0
        }) {
            return Err(format!("{} 含未知指标、负值或非法数值", coal.name));
        }
        if let Some(terms) = &coal.purchase_terms {
            validate_purchase_terms(&coal.name, terms)?;
        }
    }

    let mut indicators = std::collections::HashSet::new();
    for spec in request.specs.iter().filter(|item| item.enabled) {
        if !INDICATORS.contains(&spec.indicator.as_str()) {
            return Err(format!("未知合同指标: {}", spec.indicator));
        }
        if !indicators.insert(spec.indicator.as_str()) {
            return Err(format!("合同指标重复: {}", spec.indicator));
        }
        let needs_max = matches!(spec.direction, Direction::Upper | Direction::Range);
        let needs_min = matches!(spec.direction, Direction::Lower | Direction::Range);
        if needs_max && spec.max.is_none() {
            return Err(format!("{} 缺少上限", label_zh(&spec.indicator)));
        }
        if needs_min && spec.min.is_none() {
            return Err(format!("{} 缺少下限", label_zh(&spec.indicator)));
        }
        if spec
            .min
            .into_iter()
            .chain(spec.max)
            .any(|bound| !bound.is_finite())
        {
            return Err(format!(
                "{} 合同边界必须是有限数",
                label_zh(&spec.indicator)
            ));
        }
        if spec
            .min
            .zip(spec.max)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(format!("{} 下限大于上限", label_zh(&spec.indicator)));
        }
        if spec
            .margin
            .is_some_and(|margin| !margin.is_finite() || margin < 0.0)
        {
            return Err(format!("{} 安全余量非法", label_zh(&spec.indicator)));
        }
        if let Some(rule) = &spec.acceptance {
            if (rule.mode != AcceptanceMode::Raw && rule.decimals.is_none())
                || rule.decimals.is_some_and(|decimals| decimals > 6)
                || !rule.tolerance.is_finite()
                || rule.tolerance < 0.0
            {
                return Err(format!("{} 判定规则非法", label_zh(&spec.indicator)));
            }
        }
        if spec.enforcement == Enforcement::Priced {
            if spec.direction == Direction::Range {
                return Err(format!(
                    "{} 区间型指标不支持计价",
                    label_zh(&spec.indicator)
                ));
            }
            let Some(penalty) = &spec.penalty else {
                return Err(format!(
                    "{} 启用计价但缺扣款条款",
                    label_zh(&spec.indicator)
                ));
            };
            // Range 已在上面提前返回, 这里只剩 Upper/Lower; needs_max/needs_min 检查
            // (:293-298) 已保证对应边界存在, Some(..) else 只是防御式兜底.
            let bound = if spec.direction == Direction::Upper {
                spec.max
            } else {
                spec.min
            };
            let Some(bound) = bound else {
                return Err(format!("{} 计价缺少合同边界", label_zh(&spec.indicator)));
            };
            validate_penalty(&spec.indicator, spec.direction, bound, penalty)?;
        }
    }
    Ok(())
}

/// 校验单条采购合同计价条款 (买入侧). 规则与卖出侧 `validate_penalty` 共用,
/// 额外校验 guarantee 有限、指标已知且不重复、方向不为 Range, 以及水分折算参数的合理范围.
fn validate_purchase_terms(coal_name: &str, terms: &PurchaseTerms) -> Result<(), String> {
    let mut indicators = std::collections::HashSet::new();
    for clause in &terms.clauses {
        if !INDICATORS.contains(&clause.indicator.as_str()) {
            return Err(format!(
                "{} 采购条款指标未知: {}",
                coal_name, clause.indicator
            ));
        }
        if !indicators.insert(clause.indicator.as_str()) {
            // 同一指标两条条款会在 effective_cif 里被各自的扣款重复计算,
            // 让煤价被低估, LP 因此过量买入该煤 (同类问题参见凸性检查).
            return Err(format!(
                "{} 采购条款指标重复: {}",
                coal_name, clause.indicator
            ));
        }
        if clause.direction == Direction::Range {
            return Err(format!(
                "{} {} 区间型指标不支持采购计价",
                coal_name,
                label_zh(&clause.indicator)
            ));
        }
        if !clause.guarantee.is_finite() {
            return Err(format!(
                "{} {} 采购保证值必须是有限数",
                coal_name,
                label_zh(&clause.indicator)
            ));
        }
        validate_penalty(
            &clause.indicator,
            clause.direction,
            clause.guarantee,
            &clause.penalty,
        )
        .map_err(|err| format!("{coal_name} {err}"))?;
    }
    // 真实合同对水分二选一: 扣量(结算量 = 净重 ×(1−M实)/(1−M合))或扣价(元/吨).
    // 两者同时配置会让每吨水分被扣两遍 —— 折价与折量各算一次, 账面数字仍然合理,
    // 不会有任何报错. 全局模板一旦同时带上, 整个煤池被系统性低估.
    if terms.contract_moisture.is_some()
        && terms.clauses.iter().any(|clause| clause.indicator == "M")
    {
        return Err(format!(
            "{coal_name} 水分不能同时按扣量和扣价计: 合同水分(contract_moisture)是扣量, \
             水分条款是扣价, 两种机制互斥, 请只保留一种"
        ));
    }
    if terms
        .contract_moisture
        .is_some_and(|moisture| !moisture.is_finite() || !(0.0..=100.0).contains(&moisture))
    {
        return Err(format!("{coal_name} 合同水分必须在 0~100 之间"));
    }
    if terms
        .moisture_excess_double_threshold
        .is_some_and(|threshold| !threshold.is_finite() || !(0.0..=100.0).contains(&threshold))
    {
        return Err(format!("{coal_name} 水分双倍阈值必须在 0~100 之间"));
    }
    Ok(())
}

/// 校验计价条款. 凸性(rate 递增)与拒收线方向是安全性的核心:
/// 前者错会让 LP 低估扣款, 后者错会让计价区间为空.
///
/// `bound` 是调用方按方向解出的合同边界 (Upper 传 max, Lower 传 min);
/// 两处调用方都已在调用前把 Range 拒掉, 也都已保证 bound 存在.
fn validate_penalty(
    indicator: &str,
    direction: Direction,
    bound: f64,
    penalty: &Penalty,
) -> Result<(), String> {
    let label = label_zh(indicator);
    if penalty.tiers.is_empty() {
        return Err(format!("{label} 扣款档位不能为空"));
    }
    if !penalty.reject.is_finite() {
        return Err(format!("{label} 拒收线必须是有限数"));
    }

    let last = penalty.tiers.len() - 1;
    let mut previous_rate = f64::NEG_INFINITY;
    for (index, tier) in penalty.tiers.iter().enumerate() {
        if !tier.rate.is_finite() {
            return Err(format!("{label} 第 {} 档扣款率必须是有限数", index + 1));
        }
        if tier.rate < 0.0 {
            return Err(format!(
                "{label} 第 {} 档扣款率不能为负 (rate={})",
                index + 1,
                tier.rate
            ));
        }
        if tier.rate <= previous_rate {
            return Err(format!(
                "{label} 第 {} 档扣款率未高于前一档: 档位必须递增才能保证凸性",
                index + 1
            ));
        }
        previous_rate = tier.rate;

        if index == last {
            if tier.width.is_some() {
                return Err(format!("{label} 末档不能有宽度上限"));
            }
        } else {
            match tier.width {
                Some(width) if width.is_finite() && width > 0.0 => {}
                _ => return Err(format!("{label} 第 {} 档宽度必须是正数", index + 1)),
            }
        }
    }

    match direction {
        Direction::Upper => {
            if penalty.reject < bound {
                return Err(format!(
                    "{label} 拒收线必须不低于合同上限 (拒收线 {} < 上限 {})",
                    penalty.reject, bound
                ));
            }
        }
        Direction::Lower => {
            if penalty.reject > bound {
                return Err(format!(
                    "{label} 拒收线必须不高于合同下限 (拒收线 {} > 下限 {})",
                    penalty.reject, bound
                ));
            }
        }
        Direction::Range => return Err(format!("{label} 区间型指标不支持计价")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 计价条款校验的共享用例 (`data/penalty_cases.json`).
    ///
    /// 同一份用例前端也跑一遍 (`doudou_blend/src/penalty.test.ts`): 前端那套
    /// 字段级提示是本函数规则的镜像, 两份实现各自演化就会出现"界面放行、求解
    /// 报错"或反过来. 用例是它们不许分叉的证据 —— 改规则时先往这份数据里加
    /// 一条, 两端会同时红.
    #[derive(serde::Deserialize)]
    struct PenaltyCase {
        name: String,
        direction: Direction,
        bound: f64,
        penalty: Penalty,
        valid: bool,
    }

    #[derive(serde::Deserialize)]
    struct PenaltyCases {
        cases: Vec<PenaltyCase>,
    }

    #[test]
    fn shared_penalty_cases_match_fixture() {
        let raw = include_str!("../data/penalty_cases.json");
        let fixture: PenaltyCases = serde_json::from_str(raw).expect("用例文件必须是合法 JSON");
        assert!(fixture.cases.len() >= 10, "用例太少, 覆盖不到每条规则");

        for case in &fixture.cases {
            let result = validate_penalty("A", case.direction, case.bound, &case.penalty);
            assert_eq!(
                result.is_ok(),
                case.valid,
                "用例「{}」: 期望 valid={}, 实际 {:?}",
                case.name,
                case.valid,
                result
            );
        }
    }

    #[test]
    fn test_truncate_upper_boundary() {
        let rule = AcceptanceRule {
            mode: AcceptanceMode::Truncate,
            decimals: Some(1),
            tolerance: 0.0,
        };
        assert!((judged_value(2.5999, &rule) - 2.5).abs() < 1e-9);
        assert!((judged_value(79.999_999_995, &rule) - 80.0).abs() < 1e-9);
        assert!((judged_value(79.999_9, &rule) - 79.9).abs() < 1e-9);
        assert!(effective_upper(2.5, &rule) < 2.6);
        assert!(effective_upper(2.5, &rule) > 2.599_9);
    }

    #[test]
    fn test_round_lower_boundary() {
        let rule = AcceptanceRule {
            mode: AcceptanceMode::Round,
            decimals: Some(1),
            tolerance: 0.0,
        };
        assert!((effective_lower(80.0, &rule) - 79.95).abs() < 1e-9);
        assert_eq!(judged_value(79.96, &rule), 80.0);
    }

    fn priced_spec(tiers: Vec<PenaltyTier>, reject: f64) -> Spec {
        Spec {
            indicator: "A".into(),
            direction: Direction::Upper,
            min: None,
            max: Some(10.0),
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Priced,
            penalty: Some(Penalty { tiers, reject }),
        }
    }

    fn request_with(spec: Spec) -> BlendRequest {
        let mut props = std::collections::HashMap::new();
        for indicator in INDICATORS {
            props.insert(indicator.to_string(), 1.0);
        }
        BlendRequest {
            coals: vec![Coal {
                name: "甲".into(),
                props,
                fob: 1000.0,
                frt: 0.0,
                petrography: None,
                purchase_terms: None,
            }],
            specs: vec![spec],
            total_quantity: None,
            truncate_decimal: false,
        }
    }

    #[test]
    fn test_priced_spec_requires_penalty() {
        let mut spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            12.0,
        );
        spec.penalty = None;
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "Priced 缺 penalty 应报错"
        );
    }

    #[test]
    fn test_penalty_rates_must_increase() {
        // 递减 rate 破坏凸性: LP 会填错档并低估扣款
        let spec = priced_spec(
            vec![
                PenaltyTier {
                    width: Some(0.5),
                    rate: 80.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 40.0,
                },
            ],
            12.0,
        );
        let err = validate_request(&request_with(spec)).expect_err("rate 递减应报错");
        assert!(err.contains("递增"), "错误信息应提及递增, 实际: {err}");
    }

    #[test]
    fn test_penalty_reject_must_be_outside_contract_bound() {
        // Upper 向 reject 落在合同界内侧 ⇒ 计价区间为空
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            9.0,
        );
        let err = validate_request(&request_with(spec)).expect_err("reject 在合同界内侧应报错");
        assert!(err.contains("拒收线"), "错误信息应提及拒收线, 实际: {err}");
    }

    #[test]
    fn test_penalty_tiers_cannot_be_empty() {
        let spec = priced_spec(vec![], 12.0);
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "空档位应报错"
        );
    }

    #[test]
    fn test_penalty_reject_must_be_finite() {
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            f64::NAN,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "拒收线为 NaN 应报错"
        );
    }

    #[test]
    fn test_penalty_tier_rate_must_be_finite_and_non_negative() {
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: -1.0,
            }],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "负扣款率应报错"
        );
    }

    #[test]
    fn test_penalty_tail_tier_must_be_unbounded() {
        let spec = priced_spec(
            vec![PenaltyTier {
                width: Some(1.0),
                rate: 80.0,
            }],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "末档必须无上限"
        );
    }

    #[test]
    fn test_priced_spec_rejects_range_direction() {
        let mut spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            12.0,
        );
        spec.direction = Direction::Range;
        spec.min = Some(5.0);
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "Range 计价 spec 应报错"
        );
    }

    #[test]
    fn test_valid_priced_spec_passes() {
        let spec = priced_spec(
            vec![
                PenaltyTier {
                    width: Some(0.5),
                    rate: 10.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 20.0,
                },
            ],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_ok(),
            "合法计价条款应通过"
        );
    }

    #[test]
    fn test_valid_priced_spec_single_tier_passes() {
        // 单档 (width: None) 是真实合同最常见形态, 例如"每超 0.1% 扣 8 元/吨"无第二档.
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_ok(),
            "单档计价条款应通过"
        );
    }

    fn priced_lower_spec(tiers: Vec<PenaltyTier>, reject: f64) -> Spec {
        Spec {
            indicator: "G".into(),
            direction: Direction::Lower,
            min: Some(10.0),
            max: None,
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Priced,
            penalty: Some(Penalty { tiers, reject }),
        }
    }

    #[test]
    fn test_penalty_lower_direction_reject_must_be_outside_contract_bound() {
        // Lower 向 reject 必须不高于合同下限, 否则计价区间为空
        let spec = priced_lower_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            11.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "Lower 向 reject 落在合同界内侧应报错"
        );
    }

    #[test]
    fn test_valid_priced_lower_spec_passes() {
        let spec = priced_lower_spec(
            vec![
                PenaltyTier {
                    width: Some(0.5),
                    rate: 10.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 20.0,
                },
            ],
            8.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_ok(),
            "合法 Lower 向计价条款应通过"
        );
    }

    fn coal_with_terms(terms: PurchaseTerms) -> Coal {
        let mut props = std::collections::HashMap::new();
        for indicator in INDICATORS {
            props.insert(indicator.to_string(), 1.0);
        }
        Coal {
            name: "乙".into(),
            props,
            fob: 1000.0,
            frt: 0.0,
            petrography: None,
            purchase_terms: Some(terms),
        }
    }

    fn request_with_purchase_terms(terms: PurchaseTerms) -> BlendRequest {
        BlendRequest {
            coals: vec![coal_with_terms(terms)],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
        }
    }

    #[test]
    fn test_purchase_clause_rejects_range_direction() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Range,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧 Range 应报错"
        );
    }

    #[test]
    fn test_purchase_clause_rejects_unknown_indicator() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "X".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧未知指标应报错"
        );
    }

    #[test]
    fn test_purchase_clause_reject_wrong_side_errors() {
        // Upper 向: reject 必须不低于保证值, 否则计价区间为空
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 9.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧 reject 落在保证值内侧应报错"
        );
    }

    #[test]
    fn test_purchase_clause_requires_finite_guarantee() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: f64::NAN,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧保证值必须是有限数"
        );
    }

    #[test]
    fn test_contract_moisture_out_of_range_errors() {
        let terms = PurchaseTerms {
            clauses: vec![],
            contract_moisture: Some(150.0),
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "合同水分超出合理范围应报错"
        );
    }

    #[test]
    fn test_moisture_excess_double_threshold_out_of_range_errors() {
        let terms = PurchaseTerms {
            clauses: vec![],
            contract_moisture: None,
            moisture_excess_double_threshold: Some(-1.0),
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "水分双倍阈值超出合理范围应报错"
        );
    }

    #[test]
    fn test_purchase_clause_rejects_duplicate_indicator() {
        // 同一指标两条条款会在 effective_cif 里被重复扣款, 低估该煤成本, LP 因此过量买入.
        let terms = PurchaseTerms {
            clauses: vec![
                PurchaseClause {
                    indicator: "A".into(),
                    direction: Direction::Upper,
                    guarantee: 10.0,
                    penalty: Penalty {
                        tiers: vec![PenaltyTier {
                            width: None,
                            rate: 80.0,
                        }],
                        reject: 12.0,
                    },
                },
                PurchaseClause {
                    indicator: "A".into(),
                    direction: Direction::Upper,
                    guarantee: 10.0,
                    penalty: Penalty {
                        tiers: vec![PenaltyTier {
                            width: None,
                            rate: 50.0,
                        }],
                        reject: 12.0,
                    },
                },
            ],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧同一指标重复应报错"
        );
    }

    #[test]
    fn test_valid_purchase_clause_single_tier_passes() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "单档买入侧计价条款应通过"
        );
    }

    #[test]
    fn test_valid_purchase_clause_passes() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![
                        PenaltyTier {
                            width: Some(0.5),
                            rate: 10.0,
                        },
                        PenaltyTier {
                            width: None,
                            rate: 20.0,
                        },
                    ],
                    reject: 12.0,
                },
            }],
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: Some(10.0),
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "合法买入侧条款应通过"
        );
    }

    /// 水分两种机制互斥: contract_moisture 是扣量(折结算量), M 条款是扣价(元/吨).
    /// 全局模板若同时带上两者, 每个煤都会被水分扣两遍, 且账面完全合理、不会报错.
    #[test]
    fn test_moisture_quantity_and_price_mechanisms_are_exclusive() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "M".into(),
                direction: Direction::Upper,
                guarantee: 8.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 50.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: None,
        };
        let error = validate_request(&request_with_purchase_terms(terms))
            .expect_err("扣量与扣价同时配置应被拒绝");
        assert!(
            error.contains("乙") && error.contains("水分"),
            "报错应点名煤种与水分, 实得 {error}"
        );
    }

    /// 只用扣量(结算量折算)——我们样本合同的写法——应通过.
    #[test]
    fn test_moisture_quantity_mechanism_alone_passes() {
        let terms = PurchaseTerms {
            clauses: Vec::new(),
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: Some(12.0),
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "单用扣量机制应通过"
        );
    }

    /// 只用扣价(元/吨 水分条款)应通过.
    #[test]
    fn test_moisture_price_mechanism_alone_passes() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "M".into(),
                direction: Direction::Upper,
                guarantee: 8.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 50.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "单用扣价机制应通过"
        );
    }
}
