use blend_kit::{
    solve_json, solve_with_evaluators, BlendRequest, EvaluatorSet, GObservation, ModelPolicy,
};
use serde_json::{json, Value};

fn solve(request: Value) -> Value {
    serde_json::from_str(&solve_json(&request.to_string())).expect("求解结果必须是合法 JSON")
}

fn coal(name: &str, sulfur: f64, moisture: f64, g_value: f64, price: f64) -> Value {
    json!({
        "name": name,
        "props": {
            "S": sulfur,
            "A": 8.0,
            "V": 22.0,
            "G": g_value,
            "Y": 15.0,
            "petro": 0.10,
            "CSR": 65.0,
            "M": moisture
        },
        "fob": price,
        "frt": 30.0
    })
}

fn sulfur_check(result: &Value) -> &Value {
    result["indicator_check"]
        .as_array()
        .expect("indicator_check 应为数组")
        .iter()
        .find(|check| check["indicator"] == "S")
        .expect("结果应包含硫指标")
}

#[test]
fn test_per_spec_truncation_accepts_25999_but_rejects_26000() {
    let spec = json!({
        "indicator": "S",
        "direction": "Upper",
        "max": 2.5,
        "acceptance": {
            "mode": "Truncate",
            "decimals": 1,
            "tolerance": 0.0
        }
    });

    let accepted = solve(json!({
        "coals": [coal("截断通过", 2.5999, 10.0, 80.0, 1000.0)],
        "specs": [spec.clone()],
        "truncate_decimal": false
    }));
    assert_eq!(accepted["ok"], true);
    assert_ne!(accepted["quality_status"], "NeedsReview");
    let check = sulfur_check(&accepted);
    assert!((check["judged_value"].as_f64().unwrap() - 2.5).abs() < 1e-9);
    assert!(
        matches!(
            check["status"].as_str(),
            Some("TolerancePass" | "Unverified")
        ),
        "实际状态: {}",
        check["status"]
    );

    let rejected = solve(json!({
        "coals": [coal("截断失败", 2.6, 10.0, 80.0, 1000.0)],
        "specs": [spec],
        "truncate_decimal": false
    }));
    assert_eq!(rejected["ok"], false);

    let raw_boundary = solve(json!({
        "coals": [coal("原值边界", 2.5, 10.0, 80.0, 1000.0)],
        "specs": [{
            "indicator": "S",
            "direction": "Upper",
            "max": 2.5,
            "acceptance": {"mode": "Raw", "tolerance": 0.0}
        }],
        "truncate_decimal": false
    }));
    assert_eq!(raw_boundary["ok"], true, "原值等于上限时应通过");
}

#[test]
fn test_reporting_boundaries_support_zero_to_six_decimals() {
    for decimals in 0..=6 {
        let scale = 10_f64.powi(decimals);
        let quantum = 1.0 / scale;

        let truncate_threshold = ((2.5 * scale).floor() + 1.0) / scale;
        let truncate_accepted = truncate_threshold - quantum * 0.5;
        for (value, expected) in [(truncate_accepted, true), (truncate_threshold, false)] {
            let result = solve(json!({
                "coals": [coal("截断边界", value, 10.0, 80.0, 1000.0)],
                "specs": [{
                    "indicator": "S",
                    "direction": "Upper",
                    "max": 2.5,
                    "acceptance": {
                        "mode": "Truncate",
                        "decimals": decimals,
                        "tolerance": 0.0
                    }
                }],
                "truncate_decimal": false
            }));
            assert_eq!(
                result["ok"], expected,
                "Truncate decimals={decimals}, value={value:.9}"
            );
        }

        let round_threshold = ((2.5 * scale).floor() + 0.5) / scale;
        let round_accepted = round_threshold - quantum * 0.25;
        for (value, expected) in [(round_accepted, true), (round_threshold, false)] {
            let result = solve(json!({
                "coals": [coal("四舍五入边界", value, 10.0, 80.0, 1000.0)],
                "specs": [{
                    "indicator": "S",
                    "direction": "Upper",
                    "max": 2.5,
                    "acceptance": {
                        "mode": "Round",
                        "decimals": decimals,
                        "tolerance": 0.0
                    }
                }],
                "truncate_decimal": false
            }));
            assert_eq!(
                result["ok"], expected,
                "Round decimals={decimals}, value={value:.9}"
            );
        }
    }
}

#[test]
fn test_advisory_constraint_reports_failure_without_blocking_recipe() {
    let result = solve(json!({
        "coals": [coal("高硫参考煤", 9.0, 10.0, 80.0, 1000.0)],
        "specs": [{
            "indicator": "S",
            "direction": "Upper",
            "max": 2.5,
            "enforcement": "Advisory",
            "acceptance": {"mode": "Raw", "tolerance": 0.0}
        }],
        "truncate_decimal": false
    }));

    assert_eq!(result["ok"], true);
    assert_eq!(sulfur_check(&result)["status"], "Fail");
}

#[test]
fn test_legacy_json_remains_compatible() {
    let result = solve(json!({
        "coals": [coal("旧请求", 2.0, 10.0, 80.0, 1000.0)],
        "specs": [{"indicator": "S", "direction": "Upper", "max": 2.5}],
        "total_quantity": null,
        "truncate_decimal": true
    }));

    assert_eq!(result["ok"], true);
    assert!(result["quality_status"].is_string());
}

#[test]
fn test_invalid_quantity_and_duplicate_names_are_rejected() {
    let negative_quantity = solve(json!({
        "coals": [coal("合法煤", 2.0, 10.0, 80.0, 1000.0)],
        "specs": [],
        "total_quantity": -100.0
    }));
    assert_eq!(negative_quantity["ok"], false);

    let duplicate_names = solve(json!({
        "coals": [
            coal("同名煤", 1.0, 10.0, 80.0, 1000.0),
            coal("同名煤", 3.0, 10.0, 90.0, 1100.0)
        ],
        "specs": []
    }));
    assert_eq!(duplicate_names["ok"], false);
}

#[test]
fn test_hard_g_model_cannot_optimize_outside_training_domain() {
    let observations: Vec<GObservation> = (0..20)
        .map(|index| {
            let linear = 70.0 + index as f64 * 0.5;
            GObservation {
                g_linear: linear,
                g_measured: linear * 0.9,
            }
        })
        .collect();
    let policy = ModelPolicy {
        min_g_samples: 20,
        min_csr_samples: 30,
        max_g_cv_mae: 0.1,
        max_csr_cv_mae: 5.0,
        extrapolation_ratio: 0.0,
        ridge_lambda: 0.001,
    };
    let evaluators = EvaluatorSet::train(&observations, &[], &policy).expect("G 评估器应训练成功");
    let request: BlendRequest = serde_json::from_value(json!({
        "coals": [coal("训练域外煤", 2.0, 10.0, 100.0, 1000.0)],
        "specs": [{
            "indicator": "G",
            "direction": "Lower",
            "min": 85.0,
            "enforcement": "Hard",
            "acceptance": {"mode": "Raw", "tolerance": 0.0}
        }],
        "truncate_decimal": false
    }))
    .expect("请求应可反序列化");
    let result = solve_with_evaluators(&request, &evaluators);

    assert!(!result.ok, "Hard G 不得利用训练域外的外推值满足约束");
}

#[test]
fn test_hard_petro_scalar_does_not_require_detail_input() {
    let result = solve(json!({
        "coals": [coal("标量岩相煤", 2.0, 10.0, 80.0, 1000.0)],
        "specs": [{
            "indicator": "petro",
            "direction": "Upper",
            "max": 0.15,
            "enforcement": "Hard",
            "acceptance": {"mode": "Raw", "tolerance": 0.0}
        }],
        "truncate_decimal": false
    }));

    assert_eq!(result["ok"], true);
    assert!(result["petrography_check"].is_null());
    let petro = result["indicator_check"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["indicator"] == "petro")
        .unwrap();
    assert_eq!(petro["method"], "ProvisionalLinear");
    assert_eq!(petro["status"], "Unverified");
}

#[test]
fn test_hard_petrography_exact_violation_does_not_return_recipe() {
    let mut low = coal("低反射", 2.0, 10.0, 70.0, 1000.0);
    low["props"]["petro"] = json!(0.05);
    low["petrography"] = json!({
        "hist": [[0.85, 1.0], [0.95, 1.0]],
        "vitrinite_pct": 80.0
    });
    let mut high = coal("高反射", 2.0, 10.0, 100.0, 1100.0);
    high["props"]["petro"] = json!(0.05);
    high["petrography"] = json!({
        "hist": [[1.55, 1.0], [1.65, 1.0]],
        "vitrinite_pct": 80.0
    });
    let result = solve(json!({
        "coals": [low, high],
        "specs": [
            {"indicator": "G", "direction": "Lower", "min": 85.0},
            {
                "indicator": "petro",
                "direction": "Upper",
                "max": 0.2,
                "enforcement": "Hard",
                "acceptance": {"mode": "Raw", "tolerance": 0.0}
            }
        ],
        "truncate_decimal": false
    }));

    assert_eq!(
        result["ok"], false,
        "精确岩相修复失败后不得把违约候选当成功配方"
    );
}

#[test]
fn test_hard_petrography_exact_lower_violation_does_not_return_recipe() {
    let mut coal = coal("岩相代理偏高煤", 2.0, 10.0, 80.0, 1000.0);
    coal["props"]["petro"] = json!(0.30);
    coal["petrography"] = json!({
        "hist": [[1.00, 1.0], [1.10, 1.0]],
        "vitrinite_pct": 80.0
    });
    let result = solve(json!({
        "coals": [coal],
        "specs": [{
            "indicator": "petro",
            "direction": "Lower",
            "min": 0.20,
            "enforcement": "Hard",
            "acceptance": {"mode": "Raw", "tolerance": 0.0}
        }],
        "truncate_decimal": false
    }));

    assert_eq!(
        result["ok"], false,
        "岩相代理通过但精确 σ 低于 Hard 下限时不得返回配方"
    );
}

#[test]
fn test_advisory_only_result_is_estimated_not_verified() {
    let result = solve(json!({
        "coals": [coal("参考煤", 2.0, 10.0, 80.0, 1000.0)],
        "specs": [{
            "indicator": "S",
            "direction": "Upper",
            "max": 2.5,
            "enforcement": "Advisory"
        }],
        "truncate_decimal": false
    }));
    assert_eq!(result["ok"], true);
    assert_eq!(result["quality_status"], "Estimated");
}
