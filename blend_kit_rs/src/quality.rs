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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
