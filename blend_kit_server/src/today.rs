//! 按线上保存的状态组装"今日"求解请求 —— 网页今日屏点求解时发的那份请求的服务端版本,
//! 供 MCP (手机 Agent) 等不经过浏览器的入口使用.
//!
//! 规则逐条对照前端 `domain/resolvedCoal.ts` 的 `resolveCoal` / `resolveCoalPool` /
//! `toEstimateCoal`, 存储层 `storage.ts` 的 `getCoalPrefs` / `getUserCoals`, 与今日屏
//! `TodayScreen.tsx` 的请求拼装; 两端共用 `data/pool_cases.json` 的用例
//! (见本模块测试与 `resolvedCoal.parity.test.ts`):
//! - 自建煤里名字为空、status 不认识、`props` 不是对象的直接丢掉, 不进煤池也不参与判重名;
//! - 已停用 (archived) 一票否决, 偏好拉不回来; 隐藏的滤掉; 未启用的滤掉
//!   (默认只有 verified 启用, 偏好的 `enabled` 覆盖); 整条偏好不是对象 (含 null) 判为
//!   数据非法, 偏好里某个字段为 null 等同没写;
//! - 化验项 = master + `props_override`; master 缺 `props`、偏好结构非法、数值非法、
//!   重名 (忽略大小写与全角空格) 的煤都不进求解;
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

/// 今日请求里的 `truncate_decimal`: 合同项没写判定规则时, 核心按它决定是否一位小数截断.
pub const TRUNCATE_DECIMAL: bool = true;

/// 候选煤名最多列几个, 免得一个字匹配出半个煤库.
const MAX_CANDIDATES: usize = 10;

/// 组装结果: 请求本体, 合同名, 以及要告诉调用方的口径说明.
#[derive(Debug)]
pub struct TodayRequest {
    pub request: Value,
    pub contract_name: String,
    pub notes: Vec<String>,
    /// `exclude` 里本来会参与求解、这次被拿掉的煤 (煤库里的写法).
    pub excluded: Vec<String>,
    /// `exclude` 里点到了、但本来就不参与求解的煤, 带各自不参与的原因.
    pub excluded_idle: Vec<(String, &'static str)>,
}

/// 求解口径的可调项 (都可省).
#[derive(Debug, Default)]
pub struct TodayOptions {
    /// 覆盖保存的采购吨数.
    pub total_quantity: Option<f64>,
    /// 本次额外排除的煤 (如"雪坪停了"), 按归一化后的煤名精确匹配; 有一个对不上就不组装.
    pub exclude: Vec<String>,
    /// 验算配比: 煤名 → 份数. 给了就不求最优. 煤名按归一化后匹配, 传给核心时换成煤池里的写法;
    /// 点到对不上或不参与求解的煤就不组装.
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

/// 单煤在煤池里的状态, 与网页 `CoalReadiness` 一一对应.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Disabled,
    Hidden,
    DuplicateName,
    InvalidOverride,
}

#[cfg(test)]
impl Readiness {
    /// 网页 `CoalReadiness` 的取值.
    pub fn as_str(self) -> &'static str {
        match self {
            Readiness::Ready => "ready",
            Readiness::Disabled => "disabled",
            Readiness::Hidden => "hidden",
            Readiness::DuplicateName => "duplicate_name",
            Readiness::InvalidOverride => "invalid_override",
        }
    }
}

/// 按偏好解析后的一种煤.
#[derive(Debug)]
pub struct ResolvedCoal<'a> {
    pub entry: &'a Value,
    pub name: String,
    /// master 指标 + 偏好里改过的化验值; 非法的值不收.
    pub props: Map<String, Value>,
    /// 用户在煤池改过、且与 master 不同的指标.
    pub overridden: Vec<String>,
    pub readiness: Readiness,
    /// 不计重名时的状态: 判重名的煤原本会不会参与.
    pub own_readiness: Readiness,
}

pub fn normalize(name: &str) -> String {
    name.replace('\u{3000}', " ").trim().to_lowercase()
}

/// 自建煤的 status 只认这几种, 与网页 `storage.ts` 的 `COAL_STATUSES` 一致.
const COAL_STATUSES: [&str; 5] = ["verified", "active", "draft", "incomplete", "archived"];

/// 网页 `getUserCoals` 收下的自建煤: 煤名非空、status 认识、`props` 是对象.
fn is_valid_user_coal(entry: &Value) -> bool {
    let named = entry
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| !normalize(name).is_empty());
    let status = entry.get("status").and_then(Value::as_str);
    named
        && status.is_some_and(|s| COAL_STATUSES.contains(&s))
        && entry.get("props").is_some_and(Value::is_object)
}

/// 不参与求解的原因; 参与的煤为 None.
pub fn idle_reason(coal: &ResolvedCoal) -> Option<&'static str> {
    match coal.readiness {
        Readiness::Ready => None,
        Readiness::Disabled if coal.entry["status"] == "archived" => Some("煤矿已停用"),
        Readiness::Disabled => Some("煤池里没启用"),
        Readiness::Hidden => Some("在煤池里隐藏了"),
        Readiness::DuplicateName => Some("煤池里有重名的煤"),
        Readiness::InvalidOverride => Some("数据非法"),
    }
}

fn valid_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|x| x.is_finite() && *x >= 0.0)
}

/// 单煤解析, 逐条照网页 `resolveCoal`: 坏数据记下来但继续收能用的值, 最后按
/// 隐藏 → 未启用 → 数据非法 的顺序定状态.
fn resolve_coal<'a>(entry: &'a Value, name: &str, pref: Option<&Value>) -> ResolvedCoal<'a> {
    let status = entry.get("status").and_then(Value::as_str).unwrap_or("");
    // 网页存储层把不是对象的整条偏好 (含 null) 改写成 `{ invalid_data: true }`.
    let safe = pref.and_then(Value::as_object);
    let mut invalid = pref.is_some() && safe.is_none();
    invalid |= safe.and_then(|p| p.get("invalid_data")) == Some(&Value::Bool(true));
    let field = |key: &str| safe.and_then(|p| p.get(key)).filter(|v| !v.is_null());

    let mut enabled = status == "verified";
    if status == "archived" {
        enabled = false;
    } else if let Some(value) = field("enabled") {
        match value.as_bool() {
            Some(flag) => enabled = flag,
            None => {
                enabled = false;
                invalid = true;
            }
        }
    }
    let mut hidden = false;
    if let Some(value) = field("hidden") {
        match value.as_bool() {
            Some(flag) => hidden = flag,
            None => invalid = true,
        }
    }

    let mut props = Map::new();
    match entry.get("props").and_then(Value::as_object) {
        None => invalid = true,
        Some(base) => {
            for (key, value) in base {
                if !INDICATORS.contains(&key.as_str()) {
                    invalid = true;
                } else if !value.is_null() {
                    match valid_number(value) {
                        Some(x) => {
                            props.insert(key.clone(), json!(x));
                        }
                        None => invalid = true,
                    }
                }
            }
        }
    }
    let mut overridden = Vec::new();
    if let Some(overrides) = field("props_override") {
        match overrides.as_object() {
            None => invalid = true,
            Some(map) => {
                for (key, value) in map {
                    if !INDICATORS.contains(&key.as_str()) {
                        invalid = true;
                        continue;
                    }
                    if value.is_null() {
                        continue;
                    }
                    let Some(x) = valid_number(value) else {
                        invalid = true;
                        continue;
                    };
                    props.insert(key.clone(), json!(x));
                    if entry["props"][key.as_str()].as_f64() != Some(x) {
                        overridden.push(key.clone());
                    }
                }
            }
        }
    }

    let readiness = if hidden {
        Readiness::Hidden
    } else if !enabled {
        Readiness::Disabled
    } else if invalid {
        Readiness::InvalidOverride
    } else {
        Readiness::Ready
    };
    ResolvedCoal {
        entry,
        name: name.to_string(),
        props,
        overridden,
        readiness,
        own_readiness: readiness,
    }
}

/// 解析整个煤池: 用户新增的煤在前, 与网页一致; 重名的整组记为 [`Readiness::DuplicateName`].
/// `master` 是合并了覆盖层的煤库 JSON.
pub fn resolve_pool<'a>(
    master: &'a Value,
    storage: &'a UserStorage,
) -> Result<Vec<ResolvedCoal<'a>>, String> {
    let empty = Map::new();
    let prefs = storage.coal_prefs.as_object().unwrap_or(&empty);
    let master_coals = master
        .get("coals")
        .and_then(Value::as_array)
        .ok_or("煤库数据缺 coals")?;
    let user_coals = storage.user_coals.as_array().map(Vec::as_slice);
    let mut pool: Vec<ResolvedCoal> = user_coals
        .unwrap_or_default()
        .iter()
        .filter(|entry| is_valid_user_coal(entry))
        .chain(master_coals)
        .filter_map(|entry| {
            let name = entry.get("name").and_then(Value::as_str)?;
            Some(resolve_coal(entry, name, prefs.get(name)))
        })
        .collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for coal in &pool {
        *counts.entry(normalize(&coal.name)).or_default() += 1;
    }
    for coal in &mut pool {
        if counts[&normalize(&coal.name)] > 1 {
            coal.readiness = Readiness::DuplicateName;
        }
    }
    Ok(pool)
}

/// 名字对不上煤池里任何一种煤时的一行说明: 点出叫不上的名字, 列出名字里带它的煤.
/// `ready_only` 时只列能参与求解的煤 (验算配比只能用它们).
fn unknown_name_line(pool: &[ResolvedCoal], name: &str, ready_only: bool) -> String {
    let key = normalize(name);
    let mut candidates: Vec<&str> = Vec::new();
    let mut idle_only = false;
    for coal in pool {
        if !normalize(&coal.name).contains(&key) || candidates.contains(&coal.name.as_str()) {
            continue;
        }
        if ready_only && coal.readiness != Readiness::Ready {
            idle_only = true;
        } else {
            candidates.push(&coal.name);
        }
    }
    if candidates.is_empty() {
        if idle_only {
            return format!(
                "没有叫「{name}」的煤; 名字里带「{name}」的煤都不参与求解, 可以用 find_coal 查原因"
            );
        }
        return format!(
            "没有叫「{name}」的煤, 名字里带「{name}」的也没有; 可以用 find_coal 按煤名的一段查一下"
        );
    }
    let more = candidates.len().saturating_sub(MAX_CANDIDATES);
    candidates.truncate(MAX_CANDIDATES);
    let tail = if more > 0 {
        format!(" 等 (另有 {more} 种)")
    } else {
        String::new()
    };
    format!(
        "没有叫「{name}」的煤。名字里带「{name}」的有: {}{tail}",
        candidates.join("、")
    )
}

/// 点到的煤在煤池里、但不参与求解: 说明原因, 重名时列出撞名的几种.
fn idle_name_line(name: &str, same: &[&ResolvedCoal]) -> String {
    let reason = idle_reason(same[0]).unwrap_or("不参与求解");
    if same[0].readiness == Readiness::DuplicateName {
        let names: Vec<&str> = same.iter().map(|coal| coal.name.as_str()).collect();
        format!(
            "「{name}」在煤池里但不参与求解: {reason} ({})",
            names.join("、")
        )
    } else {
        format!("「{name}」在煤池里但不参与求解: {reason}")
    }
}

/// 煤名对不上或点到不参与求解的煤时攒下的说明.
#[derive(Default)]
struct NameProblems {
    lines: Vec<String>,
    unknown: bool,
    idle: bool,
}

impl NameProblems {
    fn into_message(mut self) -> Option<String> {
        if self.lines.is_empty() {
            return None;
        }
        let mut tail = String::from("本次没有求解。");
        if self.unknown {
            tail.push_str("请给出完整煤名; 一个词对应几种煤时, 要用哪几种就逐个列出完整煤名.");
        }
        if self.idle {
            tail.push_str("不参与求解的煤不能放进验算配比.");
        }
        self.lines.push(tail);
        Some(self.lines.join("\n"))
    }
}

/// 把验算配比的煤名换成煤池里的写法; 对不上、不参与求解或两项指向同一种煤的, 记进 `problems`.
fn map_ratio_names(
    pool: &[ResolvedCoal],
    ratios: &Map<String, Value>,
    problems: &mut NameProblems,
) -> Map<String, Value> {
    let mut mapped = Map::new();
    let mut given: HashMap<&str, &str> = HashMap::new();
    for (name, share) in ratios {
        let key = normalize(name);
        if key.is_empty() {
            problems.lines.push("配比里有一项煤名是空的".into());
            problems.unknown = true;
            continue;
        }
        let same: Vec<&ResolvedCoal> = pool
            .iter()
            .filter(|coal| normalize(&coal.name) == key)
            .collect();
        if same.is_empty() {
            problems.lines.push(unknown_name_line(pool, name, true));
            problems.unknown = true;
            continue;
        }
        let Some(coal) = same.iter().find(|coal| coal.readiness == Readiness::Ready) else {
            problems.lines.push(idle_name_line(name, &same));
            problems.idle = true;
            continue;
        };
        match given.insert(&coal.name, name) {
            Some(previous) => problems.lines.push(format!(
                "配比里「{previous}」和「{name}」是同一种煤「{}」, 请合并成一项",
                coal.name
            )),
            None => {
                mapped.insert(coal.name.clone(), share.clone());
            }
        }
    }
    mapped
}

/// 按线上保存的状态组装求解请求. `master` 是合并了覆盖层的煤库 JSON;
/// `market` 是当前生效的基准报价 (见 [`crate::benchmarks`]) 与退路市场价.
///
/// `exclude` 或 `fixed_ratios` 里有煤名对不上煤池里任何一种煤 (归一化后精确比较) 时
/// 返回 Err, 说明里列出近似的煤名 —— 不能把"没排除掉"当成"排除了"去求解.
/// 验算配比点到不参与求解的煤时同样返回 Err, 说明原因.
pub fn build(
    master: &Value,
    storage: &UserStorage,
    market: &Market,
    options: &TodayOptions,
) -> Result<TodayRequest, String> {
    let pool = resolve_pool(master, storage)?;
    let known: Vec<String> = pool.iter().map(|coal| normalize(&coal.name)).collect();
    let mut problems = NameProblems::default();
    let mut seen: Vec<&String> = Vec::new();
    for name in &options.exclude {
        let key = normalize(name);
        if !key.is_empty() && !known.contains(&key) && !seen.contains(&name) {
            seen.push(name);
            problems.lines.push(unknown_name_line(&pool, name, false));
            problems.unknown = true;
        }
    }
    let fixed_ratios = options
        .fixed_ratios
        .as_ref()
        .map(|ratios| map_ratio_names(&pool, ratios, &mut problems));
    if let Some(message) = problems.into_message() {
        return Err(message);
    }
    let excluded_keys: Vec<String> = options
        .exclude
        .iter()
        .map(|n| normalize(n))
        .filter(|k| !k.is_empty())
        .collect();

    let mut estimated = Vec::new();
    let mut skipped_invalid = Vec::new();
    let mut skipped_duplicate: Vec<String> = Vec::new();
    let mut excluded = Vec::new();
    let mut excluded_idle = Vec::new();
    for coal in &pool {
        if excluded_keys.contains(&normalize(&coal.name)) {
            if coal.readiness == Readiness::Ready {
                if !excluded.contains(&coal.name) {
                    excluded.push(coal.name.clone());
                }
            } else if let Some(reason) = idle_reason(coal) {
                if !excluded_idle.iter().any(|(name, _)| name == &coal.name) {
                    excluded_idle.push((coal.name.clone(), reason));
                }
            }
            continue;
        }
        match coal.readiness {
            Readiness::Ready => estimated.push(request_coal(coal)),
            Readiness::InvalidOverride => skipped_invalid.push(coal.name.clone()),
            Readiness::DuplicateName
                if matches!(
                    coal.own_readiness,
                    Readiness::Ready | Readiness::InvalidOverride
                ) && !skipped_duplicate.contains(&coal.name) =>
            {
                skipped_duplicate.push(coal.name.clone())
            }
            Readiness::DuplicateName | Readiness::Disabled | Readiness::Hidden => {}
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
        "truncate_decimal": TRUNCATE_DECIMAL,
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
    if let Some(ratios) = fixed_ratios {
        request["fixed_ratios"] = Value::Object(ratios);
    }

    let mut notes = Vec::new();
    if !skipped_invalid.is_empty() {
        notes.push(format!("数据非法未参与: {}", skipped_invalid.join("、")));
    }
    if !skipped_duplicate.is_empty() {
        notes.push(format!(
            "煤池里有重名的煤, 整组未参与: {}",
            skipped_duplicate.join("、")
        ));
    }
    let prefs = storage.coal_prefs.as_object();
    if prefs.into_iter().flat_map(|p| p.values()).any(|p| {
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
        excluded,
        excluded_idle,
    })
}

/// 进求解的煤: 不带报价, 核心按估算价并入煤池.
fn request_coal(coal: &ResolvedCoal) -> Value {
    let mut out = json!({ "name": coal.name, "props": coal.props, "fob": 0.0, "frt": 0.0 });
    // 煤种、形态、地区不进 LP, 估价按它们逐层对标基准.
    for key in ["coal_type", "form", "region"] {
        if let Some(text) = coal.entry.get(key).and_then(Value::as_str) {
            out[key] = json!(text);
        }
    }
    out
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
    #[test]
    fn test_unknown_exclude_name_is_refused_with_candidates() {
        let mut m = master();
        let coals = m["coals"].as_array_mut().unwrap();
        coals.push(json!({"name": "甲矿一号", "status": "verified", "props": {"S": 0.5}}));
        coals.push(json!({"name": "甲矿二号", "status": "archived", "props": {"S": 0.5}}));
        let err = build(
            &m,
            &storage(json!({})),
            &index_only(1907.0),
            &TodayOptions {
                exclude: vec!["甲矿".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("没有叫「甲矿」的煤"), "{err}");
        assert!(
            err.contains("名字里带「甲矿」的有: 甲矿一号、甲矿二号"),
            "{err}"
        );
        assert!(err.contains("完整煤名"), "{err}");

        let err = build(
            &m,
            &storage(json!({})),
            &index_only(1907.0),
            &TodayOptions {
                exclude: vec!["有价".into(), "乙矿".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("没有叫「乙矿」的煤"), "{err}");
        assert!(err.contains("find_coal"), "没有近似的煤要提示去查: {err}");
        assert!(!err.contains("「有价」"), "对得上的名字不该点名: {err}");
    }

    #[test]
    fn test_exclude_reports_only_coals_that_would_have_joined() {
        let mut m = master();
        m["coals"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "备用", "status": "verified", "props": {"S": 0.5}}));
        let built = build(
            &m,
            &storage(json!({})),
            &index_only(1907.0),
            &TodayOptions {
                // 归一化后比较: 大小写、全角空格、首尾空格不算差别.
                exclude: vec!["　有价 ".into(), "没启用".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(built.excluded, ["有价"]);
        assert_eq!(
            built.excluded_idle,
            [("没启用".to_string(), "煤池里没启用")]
        );
        assert_eq!(
            names(&built.request["price_factor"]["extra_coals"]),
            ["备用"]
        );
        // 排除只靠不放进煤池, 调用方写的煤名不进请求.
        assert!(built.request.get("exclude").is_none());
    }

    #[test]
    fn test_unknown_ratio_name_is_refused() {
        let mut ratios = Map::new();
        ratios.insert("有价".into(), json!(1));
        ratios.insert("价".into(), json!(1));
        let err = build(
            &master(),
            &storage(json!({})),
            &index_only(1907.0),
            &TodayOptions {
                fixed_ratios: Some(ratios),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("没有叫「价」的煤"), "{err}");
        // 验算只能用参与求解的煤: 没启用的无价不当候选.
        assert!(err.contains("名字里带「价」的有: 有价"), "{err}");
        assert!(!err.contains("无价"), "{err}");

        let mut ratios = Map::new();
        ratios.insert("没启".into(), json!(1));
        let err = build(
            &master(),
            &storage(json!({})),
            &index_only(1907.0),
            &TodayOptions {
                fixed_ratios: Some(ratios),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("名字里带「没启」的煤都不参与求解"), "{err}");
    }

    fn ratios_of(pairs: &[(&str, f64)]) -> TodayOptions {
        TodayOptions {
            fixed_ratios: Some(
                pairs
                    .iter()
                    .map(|(name, share)| (name.to_string(), json!(share)))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn test_ratio_names_are_passed_as_pool_names() {
        let mut m = master();
        m["coals"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "ABC", "status": "verified", "props": {"S": 0.5}}));
        let built = build(
            &m,
            &storage(json!({})),
            &index_only(1907.0),
            &ratios_of(&[("abc ", 2.0), ("　有价", 1.0)]),
        )
        .unwrap();
        assert_eq!(
            built.request["fixed_ratios"],
            json!({"ABC": 2.0, "有价": 1.0})
        );

        let err = build(
            &m,
            &storage(json!({})),
            &index_only(1907.0),
            &ratios_of(&[("ABC", 1.0), ("abc", 1.0)]),
        )
        .unwrap_err();
        assert!(err.contains("是同一种煤"), "{err}");
    }

    #[test]
    fn test_ratio_on_idle_coal_names_the_reason() {
        let mut m = master();
        m["coals"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "有价 ", "status": "verified", "props": {"S": 1.0}}));
        let prefs = json!({"无价": {"hidden": true}, "没启用": null});
        let err = build(
            &m,
            &storage(prefs),
            &index_only(1907.0),
            &ratios_of(&[("停了", 1.0), ("无价", 1.0), ("有价", 1.0), ("没启用", 1.0)]),
        )
        .unwrap_err();
        assert!(
            err.contains("「停了」在煤池里但不参与求解: 煤矿已停用"),
            "{err}"
        );
        assert!(
            err.contains("「无价」在煤池里但不参与求解: 在煤池里隐藏了"),
            "{err}"
        );
        assert!(
            err.contains("「有价」在煤池里但不参与求解: 煤池里有重名的煤 (有价、有价 )"),
            "{err}"
        );
        assert!(
            err.contains("「没启用」在煤池里但不参与求解: 煤池里没启用"),
            "{err}"
        );
        assert!(!err.contains("无解"), "{err}");
        assert!(err.contains("不参与求解的煤不能放进验算配比"), "{err}");
    }

    #[test]
    fn test_pool_marks_overridden_props() {
        let prefs = json!({"有价": {"props_override": {"S": 1.0, "A": 9.5}}});
        let s = storage(prefs);
        let m = master();
        let pool = resolve_pool(&m, &s).unwrap();
        let coal = pool.iter().find(|c| c.name == "有价").unwrap();
        // S 与 master 相同, 不算改过.
        assert_eq!(coal.overridden, ["A"]);
        assert_eq!(coal.props["A"], 9.5);
    }
    /// 煤池解析的共享用例 (`data/pool_cases.json`), 网页 `resolvedCoal.parity.test.ts` 跑同一份.
    #[test]
    fn shared_pool_cases_match_fixture() {
        let fixture: Value = serde_json::from_str(include_str!("../data/pool_cases.json")).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 12);
        for case in cases {
            let label = case["name"].as_str().unwrap();
            let master = json!({
                "price_factor": {"adjusts": []},
                "coals": case["master_coals"],
            });
            let mut s = storage(case["prefs"].clone());
            s.user_coals = case["user_coals"].clone();
            let expected: Vec<(String, String)> =
                serde_json::from_value(case["expected"].clone()).unwrap();
            let pool = resolve_pool(&master, &s).unwrap();
            let resolved: Vec<(String, String)> = pool
                .iter()
                .map(|c| (c.name.clone(), c.readiness.as_str().to_string()))
                .collect();
            assert_eq!(resolved, expected, "{label}");
            for coal in &pool {
                let mut got = coal.overridden.clone();
                got.sort();
                let want: Vec<String> =
                    serde_json::from_value(case["overridden"][coal.name.as_str()].clone())
                        .unwrap_or_default();
                assert_eq!(got, want, "{label}: {} 改过的指标", coal.name);
            }
            let ready: Vec<&str> = expected
                .iter()
                .filter(|(_, readiness)| readiness == "ready")
                .map(|(name, _)| name.as_str())
                .collect();
            let built = build(&master, &s, &index_only(1907.0), &TodayOptions::default());
            if ready.is_empty() {
                assert!(built.is_err(), "{label}: 没有可参与的煤应报错");
                continue;
            }
            let built = built.unwrap_or_else(|e| panic!("{label}: {e}"));
            let extra = &built.request["price_factor"]["extra_coals"];
            assert_eq!(names(extra), ready, "{label}");
            for coal in extra.as_array().unwrap() {
                let name = coal["name"].as_str().unwrap();
                let got: HashMap<String, f64> =
                    serde_json::from_value(coal["props"].clone()).unwrap();
                let want: HashMap<String, f64> =
                    serde_json::from_value(case["props"][name].clone()).unwrap();
                assert_eq!(got, want, "{label}: {name} 的指标");
            }
            // 数据非法的煤要在提示里点名.
            for (name, readiness) in &expected {
                if readiness == "invalid_override" {
                    assert!(
                        built
                            .notes
                            .iter()
                            .any(|n| n.contains("数据非法") && n.contains(name.as_str())),
                        "{label}: {name} 应在提示里点名, 实得 {:?}",
                        built.notes
                    );
                }
            }
        }
    }
}
