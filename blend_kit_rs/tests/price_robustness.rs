//! 价格稳健性 (`BlendRequest.price_robustness`): 逐煤翻转门槛 + 更稳的配方.
//!
//! 全部走公开 JSON 入口 `solve_json`, 数据都是 fixture, 不读 master.
//! 煤池只让硫分有差别, 拐点可以手算: 合同 S ≤ 1.0, 低硫煤 S0.5 与高硫煤 S1.5 各半.

use blend_kit::*;
use serde_json::{json, Value};

fn coal(name: &str, s: f64, fob: f64) -> Value {
    json!({
        "name": name, "fob": fob, "frt": 0.0,
        "props": {"S": s, "A": 9.0, "V": 25.0, "G": 85.0, "Y": 15.0, "petro": 0.1, "CSR": 65.0, "M": 10.0}
    })
}

fn request(coals: Vec<Value>) -> Value {
    json!({
        "coals": coals,
        "specs": [{"indicator": "S", "direction": "Upper", "max": 1.0, "enabled": true}],
        "total_quantity": 1000.0,
        "truncate_decimal": false
    })
}

fn run(req: &Value) -> Value {
    serde_json::from_str(&solve_json(&req.to_string())).unwrap()
}

fn swing<'a>(result: &'a Value, name: &str) -> &'a Value {
    result["price_robustness"]["swings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|swing| swing["name"] == name)
        .unwrap_or_else(|| panic!("{name} 不在翻转门槛表里"))
}

/// 门槛在 5 元网格上二分, 报最小的"已经变了"的格点: 5 的倍数, 落在 (拐点, 拐点 + 5].
/// 拐点故意不取 5 的倍数, 免得格点恰好碰上并列.
fn assert_threshold(value: &Value, breakpoint: f64) {
    let threshold = value
        .as_f64()
        .unwrap_or_else(|| panic!("门槛为空, 期望约 {breakpoint}"));
    assert!(
        threshold > breakpoint && threshold <= breakpoint + 5.0 && threshold % 5.0 == 0.0,
        "门槛 {threshold} 不是 ({breakpoint}, {}] 里的 5 的倍数",
        breakpoint + 5.0
    );
}

/// 低硫甲 1000, 高硫乙 903, 低硫丙 1052 (没入选). 各半配出 S1.0.
fn three_coals() -> Value {
    request(vec![
        coal("甲", 0.5, 1000.0),
        coal("乙", 1.5, 903.0),
        coal("丙", 0.5, 1052.0),
    ])
}

#[test]
fn test_absent_config_adds_no_report() {
    let result = run(&three_coals());
    assert_eq!(result["ok"], true);
    assert!(result.get("price_robustness").is_none());
}

#[test]
fn test_swing_thresholds_match_hand_computed_breakpoints() {
    let mut req = three_coals();
    req["price_robustness"] = json!({});
    let result = run(&req);
    assert_eq!(result["ok"], true);

    // 甲涨过 52 就被丙顶替; 跌过 97 (低于乙) 就单用甲.
    let jia = swing(&result, "甲");
    assert!((jia["share"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert_threshold(&jia["rise"], 52.0);
    assert_threshold(&jia["fall"], 97.0);
    // 乙涨过 97 (贵过甲) 就单用甲; 跌多少都受硫分卡着, 配方不变.
    let yi = swing(&result, "乙");
    assert_threshold(&yi["rise"], 97.0);
    assert!(yi["fall"].is_null());
    // 丙没入选: 涨价无影响; 跌过 52 (便宜过甲) 就顶替甲.
    let bing = swing(&result, "丙");
    assert_eq!(bing["share"], 0.0);
    assert!(bing["rise"].is_null());
    assert_threshold(&bing["fall"], 52.0);

    // 都有报价, 用默认报价误差 30: 门槛都大于误差, 没有要先问价的;
    // 按门槛小的在前, 同门槛占比大的在前.
    assert_eq!(jia["estimated"], false);
    assert_eq!(jia["uncertainty"], 30.0);
    assert_eq!(jia["ask"], false);
    let order: Vec<&str> = result["price_robustness"]["swings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|swing| swing["name"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["甲", "丙", "乙"]);
}

/// 乙 900 与 稳 (有报价, 1004) 各半; 估 (无报价, 估算价 1000) 是同质量的替代.
/// 只看估算价, 估比稳便宜 4 元, 配方选估; 但估的价误差大.
fn estimated_vs_quoted(steady_price: f64) -> Value {
    let mut req = request(vec![coal("乙", 1.5, 900.0), coal("稳", 0.5, steady_price)]);
    let mut guess = coal("估", 0.5, 0.0);
    guess["coal_type"] = json!("瘦煤");
    req["price_factor"] = json!({
        // 费率全 0: 每种煤都是 1000 点, 估算价 = 市场价.
        "adjusts": [{"indicator": "S", "direction": "Upper", "base": 1.3,
                     "premium_rate": 0, "discount_rate": 0}],
        "market_price": 1000.0,
        "extra_coals": [guess],
        "estimate_unpriced": true,
        "uncertainty": {"default": 200.0, "quoted": 20.0, "by_coal_type": {"瘦煤": 300.0}}
    });
    req["price_robustness"] = json!({});
    req
}

#[test]
fn test_robust_recipe_prefers_known_prices_within_tolerance() {
    let result = run(&estimated_vs_quoted(1004.0));
    assert_eq!(result["ok"], true);
    assert!((result["recipe"]["估"].as_f64().unwrap() - 0.5).abs() < 1e-6);

    let report = &result["price_robustness"];
    // 当前配方的价格误差 = 0.5 × 300 (瘦煤估算价) + 0.5 × 20 (报价).
    assert!((report["price_error"].as_f64().unwrap() - 160.0).abs() < 1e-6);
    let guess = swing(&result, "估");
    assert_eq!(guess["estimated"], true);
    assert_eq!(guess["coal_type"], "瘦煤");
    assert_eq!(guess["uncertainty"], 300.0);
    // 估贵 4 元就被稳顶替: 门槛落在网格下限 5 (即"不超过 5 元"), 远小于误差 300,
    // 占比 × 误差最大, 排第一并标先问价. 乙要贵 97 元才变, 大于误差 20, 不标.
    assert_eq!(guess["rise"], 5.0);
    assert_eq!(guess["ask"], true);
    assert_eq!(report["swings"][0]["name"], "估");
    assert_eq!(swing(&result, "乙")["ask"], false);

    // 换成稳只多花 0.5 × 4 = 2 元/吨 (容差 10 以内), 价格误差降到 20.
    let robust = &report["robust"];
    assert!((robust["recipe"]["稳"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!(robust["recipe"].get("估").is_none());
    assert!((robust["extra_cost"].as_f64().unwrap() - 2.0).abs() < 1e-6);
    assert!((robust["price_error"].as_f64().unwrap() - 20.0).abs() < 1e-6);
}

/// λ = 1 时选中的配方超出容差, 要二分出较小的 λ: 中 (估算价 1006, 误差 100) 在小 λ 下
/// 先于稳 (报价 1030, 换过去多花 15) 胜出, 只多花 3 元.
#[test]
fn test_robust_recipe_bisects_risk_premium_when_full_premium_costs_too_much() {
    let mut req = estimated_vs_quoted(1030.0);
    let mut middle = coal("中", 0.5, 0.0);
    middle["coal_type"] = json!("肥煤");
    middle["props"]["Y"] = json!(17.0); // Y 每高 1 加 3 点: 估算价 1006
    req["price_factor"]["extra_coals"]
        .as_array_mut()
        .unwrap()
        .push(middle);
    req["price_factor"]["adjusts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"indicator": "Y", "direction": "Lower", "base": 15,
                     "premium_rate": 3, "discount_rate": 3}));
    req["price_factor"]["uncertainty"]["by_coal_type"]["肥煤"] = json!(100.0);
    let result = run(&req);
    assert_eq!(result["ok"], true);
    assert!((result["recipe"]["估"].as_f64().unwrap() - 0.5).abs() < 1e-6);

    // 内点法的配比与成本残差在 1e-5 量级 (经验证算再累一层).
    let robust = &result["price_robustness"]["robust"];
    assert!(
        (robust["recipe"]["中"].as_f64().unwrap() - 0.5).abs() < 1e-3,
        "{robust}"
    );
    assert!(
        (robust["extra_cost"].as_f64().unwrap() - 3.0).abs() < 1e-3,
        "{robust}"
    );
    // 0.5 × 100 + 0.5 × 20
    assert!((robust["price_error"].as_f64().unwrap() - 60.0).abs() < 1e-3);
}

#[test]
fn test_missing_uncertainty_fields_fall_back_instead_of_rejecting() {
    let mut req = estimated_vs_quoted(1004.0);
    req["price_factor"]["uncertainty"] = json!({});
    let result = run(&req);
    assert_eq!(result["ok"], true, "缺字段不该让整个请求失败");
    assert_eq!(swing(&result, "估")["uncertainty"], 200.0);
    assert_eq!(swing(&result, "乙")["uncertainty"], 30.0);
}

#[test]
fn test_no_robust_recipe_when_alternative_costs_beyond_tolerance() {
    // 稳 1030: 换过去多花 15 元/吨, 超过容差 10.
    let result = run(&estimated_vs_quoted(1030.0));
    assert_eq!(result["ok"], true);
    let report = &result["price_robustness"];
    assert!(report["swings"]
        .as_array()
        .is_some_and(|swings| !swings.is_empty()));
    assert!(report.get("robust").is_none());
}

#[test]
fn test_invalid_config_warns_and_skips() {
    let mut req = three_coals();
    req["price_robustness"] = json!({"tolerance": -1.0});
    let result = run(&req);
    assert_eq!(result["ok"], true);
    assert!(result.get("price_robustness").is_none());
    let warned = result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("价格稳健性"));
    assert!(warned, "应提示配置无效: {}", result["warnings"]);
}

#[test]
fn test_skipped_for_fixed_ratios_and_infeasible() {
    let mut fixed = three_coals();
    fixed["price_robustness"] = json!({});
    fixed["fixed_ratios"] = json!({"甲": 1, "乙": 1});
    assert!(run(&fixed).get("price_robustness").is_none());

    let mut infeasible = request(vec![coal("乙", 1.5, 900.0)]);
    infeasible["price_robustness"] = json!({});
    let result = run(&infeasible);
    assert_eq!(result["ok"], false);
    assert!(result.get("price_robustness").is_none());
}
