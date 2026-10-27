//! 按线上保存的状态组装"今日"求解请求 —— 网页今日屏点求解时发的那份请求的服务端版本,
//! 供 MCP (手机 Agent) 等不经过浏览器的入口使用.
//!
//! 规则逐条对照前端 `domain/resolvedCoal.ts` 的 `resolveCoal` / `toEstimateCoal`,
//! 与今日屏 `TodayScreen.tsx` 的请求拼装:
//! - 已停用 (archived) 一票否决, 偏好拉不回来; 隐藏的滤掉; 未启用的滤掉
//!   (默认只有 verified 启用, 偏好的 `enabled` 覆盖);
//! - 化验项 = master + `props_override`; 偏好结构非法、数值非法、重名 (忽略大小写与
//!   全角空格) 的煤都不进求解;
//! - 单煤真实报价拿不到, 煤价一律按质量估算: 不读 master 的 fob/frt, 也不读偏好里旧版本
//!   录过的价格; 所有煤进 `price_factor.extra_coals` (fob/frt 记 0), 打开 `estimate_unpriced`.
//!   真实报价是在估算之后去谈的, 不进求解.
//!
//! 与网页的差异 (在 `notes` 里如实告诉调用方): 采购扣款模板只存在浏览器本地,
//! 服务端拿不到, 所以不计买入侧扣款.
use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::database::UserStorage;

pub const INDICATORS: [&str; 8] = ["S", "A", "V", "G", "Y", "petro", "CSR", "M"];

/// 组装结果: 请求本体, 合同名, 以及要告诉调用方的口径说明.
#[derive(Debug)]
pub struct TodayRequest {
    pub request: Value,
    pub contract_name: String,
    pub notes: Vec<String>,
}

/// 求解口径的可调项 (都可省).
#[derive(Debug, Default)]
pub struct TodayOptions {
    /// 覆盖保存的采购吨数.
    pub total_quantity: Option<f64>,
    /// 本次额外排除的煤 (如"雪坪停了"), 按煤名精确匹配.
    pub exclude: Vec<String>,
    /// 验算配比: 煤名 → 份数. 给了就不求最优.
    pub fixed_ratios: Option<Map<String, Value>>,
}

/// 估价用的市场数据.
#[derive(Debug, Default)]
pub struct Market {
    /// 当前生效的基准报价 (分煤种, 带日期). 有就按它估价.
    pub benchmarks: Vec<Value>,
    /// 退路: 没有任何基准时的标准品价 (现货指数点位, 只当相对尺度).
    pub market_price: Option<f64>,
}

fn normalize(name: &str) -> String {
    name.replace('\u{3000}', " ").trim().to_lowercase()
}

fn valid_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|x| x.is_finite() && *x >= 0.0)
}

/// 单煤解析. Ok(None) = 不参与 (停用/隐藏/未启用); Err = 数据非法, 同样不参与.
fn resolve_coal(entry: &Value, pref: Option<&Value>) -> Result<Option<Value>, ()> {
    let name = entry.get("name").and_then(Value::as_str).ok_or(())?;
    let status = entry.get("status").and_then(Value::as_str).unwrap_or("");
    if status == "archived" {
        return Ok(None);
    }
    let pref = match pref {
        None => None,
        Some(Value::Object(map)) => Some(map),
        Some(_) => return Err(()),
    };
    if pref.and_then(|p| p.get("invalid_data")) == Some(&Value::Bool(true)) {
        return Err(());
    }
    let enabled = match pref.and_then(|p| p.get("enabled")).filter(|v| !v.is_null()) {
        None => status == "verified",
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(()),
    };
    match pref.and_then(|p| p.get("hidden")).filter(|v| !v.is_null()) {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => return Ok(None),
        Some(_) => return Err(()),
    }
    if !enabled {
        return Ok(None);
    }

    let mut props = Map::new();
    if let Some(base) = entry.get("props") {
        for (key, value) in base.as_object().ok_or(())? {
            if !INDICATORS.contains(&key.as_str()) {
                return Err(());
            }
            if value.is_null() {
                continue;
            }
            props.insert(key.clone(), json!(valid_number(value).ok_or(())?));
        }
    }
    if let Some(overrides) = pref
        .and_then(|p| p.get("props_override"))
        .filter(|v| !v.is_null())
    {
        for (key, value) in overrides.as_object().ok_or(())? {
            if !INDICATORS.contains(&key.as_str()) {
                return Err(());
            }
            if value.is_null() {
                continue;
            }
            props.insert(key.clone(), json!(valid_number(value).ok_or(())?));
        }
    }

    // 不带报价: 核心按估算价并入煤池.
    let mut coal = json!({ "name": name, "props": props, "fob": 0.0, "frt": 0.0 });
    // 煤种、形态、地区不进 LP, 估价按它们逐层对标基准.
    for key in ["coal_type", "form", "region"] {
        if let Some(text) = entry.get(key).and_then(Value::as_str) {
            coal[key] = json!(text);
        }
    }
    Ok(Some(coal))
}

/// 按线上保存的状态组装求解请求. `master` 是合并了覆盖层的煤库 JSON;
/// `market` 是当前生效的基准报价 (见 [`crate::benchmarks`]) 与退路市场价.
pub fn build(
    master: &Value,
    storage: &UserStorage,
    market: &Market,
    options: &TodayOptions,
) -> Result<TodayRequest, String> {
    let empty = Map::new();
    let prefs = storage.coal_prefs.as_object().unwrap_or(&empty);
    let user_coals = storage.user_coals.as_array().cloned().unwrap_or_default();
    let master_coals = master
        .get("coals")
        .and_then(Value::as_array)
        .ok_or("煤库数据缺 coals")?;

    // 用户新增的煤在前, 与网页一致; 重名的整组排除.
    let entries: Vec<&Value> = user_coals.iter().chain(master_coals).collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for entry in &entries {
        if let Some(name) = entry.get("name").and_then(Value::as_str) {
            *counts.entry(normalize(name)).or_default() += 1;
        }
    }
    let excluded: Vec<String> = options.exclude.iter().map(|n| normalize(n)).collect();

    let mut estimated = Vec::new();
    let mut skipped_invalid = Vec::new();
    for entry in entries {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        let key = normalize(name);
        if counts.get(&key).copied().unwrap_or(0) > 1 || excluded.contains(&key) {
            continue;
        }
        match resolve_coal(entry, prefs.get(name)) {
            Ok(Some(coal)) => estimated.push(coal),
            Ok(None) => {}
            Err(()) => skipped_invalid.push(name.to_string()),
        }
    }

    let mut factor = master
        .get("price_factor")
        .and_then(Value::as_object)
        .cloned()
        .ok_or("缺少估价参数 (price_factor), 无法估算煤价")?;
    if estimated.is_empty() {
        return Err("没有可参与求解的煤: 请在煤池启用煤种".into());
    }

    let (specs, contract_name) = match storage.contract.as_ref().filter(|c| c.is_array()) {
        Some(specs) => (specs.clone(), "用户自定义合同".to_string()),
        None => (
            master["default_contract"]["specs"].clone(),
            master["default_contract"]["name"]
                .as_str()
                .unwrap_or("默认合同")
                .to_string(),
        ),
    };

    let mut request = json!({
        "coals": [],
        "specs": specs,
        "total_quantity": options.total_quantity.unwrap_or(storage.quantity),
        "shovel_tons": storage.shovel_tons.unwrap_or(crate::database::DEFAULT_SHOVEL_TONS),
        "truncate_decimal": true,
    });
    for key in crate::benchmarks::MASTER_ONLY_KEYS {
        factor.remove(key);
    }
    if !market.benchmarks.is_empty() {
        factor.insert("benchmarks".into(), json!(market.benchmarks));
    } else if let Some(price) = market.market_price {
        factor.insert("market_price".into(), json!(price));
    }
    factor.insert("extra_coals".into(), Value::Array(estimated));
    factor.insert("estimate_unpriced".into(), json!(true));
    request["price_factor"] = Value::Object(factor);
    // 不带 price_robustness: 今日页已不做估价可靠度分析, 这里与网页一致.
    if let Some(ratios) = &options.fixed_ratios {
        request["fixed_ratios"] = Value::Object(ratios.clone());
    }

    let mut notes = Vec::new();
    if !skipped_invalid.is_empty() {
        notes.push(format!("数据非法未参与: {}", skipped_invalid.join("、")));
    }
    if prefs.values().any(|p| {
        p.get("purchase_guarantees")
            .is_some_and(|g| g.as_object().is_some_and(|g| !g.is_empty()))
    }) {
        notes.push("采购扣款模板只存在浏览器, 本次未计买入侧扣款".into());
    }
    if market.benchmarks.is_empty() && market.market_price.is_none() {
        notes.push("没有基准报价, 现货指数也取不到, 估不出煤价".into());
    }
    Ok(TodayRequest {
        request,
        contract_name,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn master() -> Value {
        json!({
            "default_contract": {"name": "默认合同", "specs": [{"indicator": "S", "direction": "Upper", "max": 2.0}]},
            "price_factor": {"description": "出处", "adjusts": []},
            "coals": [
                {"name": "有价", "status": "verified", "coal_type": "焦煤", "props": {"S": 1.0}, "fob": 1000, "frt": 50},
                {"name": "无价", "status": "active", "coal_type": "瘦煤", "form": "筛选精煤", "region": "山西临汾", "props": {"S": 0.5}, "frt": 30},
                {"name": "停了", "status": "archived", "props": {"S": 0.5}},
                {"name": "没启用", "status": "active", "props": {"S": 0.5}},
            ]
        })
    }

    fn storage(prefs: Value) -> UserStorage {
        UserStorage {
            initialized: true,
            coal_prefs: prefs,
            contract: None,
            quantity: 3700.0,
            user_coals: json!([]),
            shovel_tons: None,
        }
    }

    fn index_only(price: f64) -> Market {
        Market {
            benchmarks: Vec::new(),
            market_price: Some(price),
        }
    }

    fn names(list: &Value) -> Vec<&str> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn test_pool_rules_match_web() {
        let prefs = json!({"无价": {"enabled": true}, "停了": {"enabled": true}});
        let built = build(
            &master(),
            &storage(prefs),
            &index_only(1907.0),
            &TodayOptions::default(),
        )
        .unwrap();
        let req = &built.request;
        // 煤价一律按估算: 求解煤池为空, 所有启用的煤进 extra_coals, master 里的 fob/frt 不带.
        assert_eq!(req["coals"], json!([]));
        assert_eq!(
            req["price_factor"]["extra_coals"],
            json!([
                {"name": "有价", "props": {"S": 1.0}, "fob": 0.0, "frt": 0.0, "coal_type": "焦煤"},
                {"name": "无价", "props": {"S": 0.5}, "fob": 0.0, "frt": 0.0, "coal_type": "瘦煤",
                 "form": "筛选精煤", "region": "山西临汾"}
            ])
        );
        // 与网页一致: 不做价格稳健性分析
        assert!(req.get("price_robustness").is_none());
        assert_eq!(req["price_factor"]["estimate_unpriced"], true);
        assert_eq!(req["price_factor"]["market_price"], 1907.0);
        assert!(req["price_factor"].get("description").is_none());
        assert_eq!(req["total_quantity"], 3700.0);
        // 没设过一铲吨数: 按默认 3 吨.
        assert_eq!(req["shovel_tons"], 3.0);
        assert_eq!(built.contract_name, "默认合同");
    }

    #[test]
    fn test_overrides_hidden_and_exclude() {
        let prefs = json!({
            // 旧版本录过的价格: 忽略, 不算坏数据
            "有价": {"fob_override": 900, "frt_override": 24, "props_override": {"S": 1.2}},
            "无价": {"enabled": true, "hidden": true},
        });
        let built = build(
            &master(),
            &storage(prefs),
            &Market::default(),
            &TodayOptions {
                exclude: vec![],
                total_quantity: Some(100.0),
                fixed_ratios: None,
            },
        )
        .unwrap();
        let extra = &built.request["price_factor"]["extra_coals"];
        assert_eq!(names(extra), ["有价"]);
        assert_eq!(extra[0]["fob"], 0.0);
        assert_eq!(extra[0]["frt"], 0.0);
        assert_eq!(extra[0]["props"]["S"], 1.2);
        assert_eq!(built.request["total_quantity"], 100.0);
        assert!(built.notes.iter().any(|n| n.contains("现货指数")));

        let excluded = build(
            &master(),
            &storage(json!({})),
            &Market::default(),
            &TodayOptions {
                exclude: vec!["有价".into()],
                ..Default::default()
            },
        );
        assert!(excluded.is_err(), "排除唯一可用的煤后应无煤可解");
    }

    #[test]
    fn test_no_price_factor_is_an_error() {
        let mut m = master();
        m.as_object_mut().unwrap().remove("price_factor");
        let built = build(
            &m,
            &storage(json!({})),
            &index_only(1907.0),
            &TodayOptions::default(),
        );
        assert!(built.unwrap_err().contains("估价参数"));
    }

    #[test]
    fn test_invalid_and_duplicate_coals_skipped() {
        let mut m = master();
        m["coals"].as_array_mut().unwrap().push(
            json!({"name": "有价 ", "status": "verified", "props": {"S": 1.0}, "fob": 1, "frt": 1}),
        );
        let prefs = json!({"无价": {"enabled": true, "props_override": {"S": -5}}});
        let built = build(
            &m,
            &storage(prefs),
            &index_only(1907.0),
            &TodayOptions::default(),
        );
        // 有价与"有价 "重名整组排除, 无价覆盖的化验值非法 → 无煤可解
        assert!(built.is_err());
    }

    #[test]
    fn test_benchmarks_replace_index_price() {
        let prefs = json!({"无价": {"enabled": true}});
        let market = Market {
            benchmarks: vec![json!({"name": "晋中中硫主焦", "price": 2050.0})],
            market_price: Some(1907.0),
        };
        let built = build(
            &master(),
            &storage(prefs),
            &market,
            &TodayOptions::default(),
        )
        .unwrap();
        let factor = &built.request["price_factor"];
        assert_eq!(factor["benchmarks"][0]["name"], "晋中中硫主焦");
        assert!(
            factor.get("market_price").is_none(),
            "有基准就不用指数点当价"
        );
        assert!(built.notes.iter().all(|n| !n.contains("现货指数")));
    }

    #[test]
    fn test_user_contract_and_fixed_ratios() {
        let mut s = storage(json!({}));
        s.contract = Some(json!([{"indicator": "A", "direction": "Upper", "max": 10.0}]));
        let mut ratios = Map::new();
        ratios.insert("有价".into(), json!(1));
        let built = build(
            &master(),
            &s,
            &index_only(1907.0),
            &TodayOptions {
                fixed_ratios: Some(ratios),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(built.contract_name, "用户自定义合同");
        assert_eq!(built.request["specs"][0]["indicator"], "A");
        assert_eq!(built.request["fixed_ratios"]["有价"], 1);
        assert!(
            built.request.get("price_robustness").is_none(),
            "验算不做稳健性分析"
        );
    }
}
