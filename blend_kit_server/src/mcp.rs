//! MCP (Model Context Protocol) 入口: 让手机上的 Agent (Kimi、Claude 等) 直接调配煤.
//!
//! 传输是无状态的 Streamable HTTP: 每个 POST 带一条 (或一批) JSON-RPC 消息, 直接回 JSON,
//! 不开 SSE 流, 不发会话 id —— 四个工具都是一问一答, 用不着服务端推送.
//!
//! 求解请求一律由 [`crate::today`] 按线上保存的合同与煤池组装, Agent 不必理解请求格式,
//! 算出来的与网页今日屏同一口径。本模块只做协议分派与结果排版, 不碰库也不联网 ——
//! 数据由调用方装进 [`Context`] 再传进来, 所以全部可以用固定数据单测。
use serde_json::{json, Map, Value};

use crate::database::UserStorage;
use crate::today::{self, Market, TodayOptions};

/// 支持的协议版本, 新的在前. 客户端要的版本在列表里就照回, 否则回最新的.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "豆哥配煤: 炼焦配煤的最低成本配方。所有煤价都是估算价: 按同煤种、指标最接近的 \
基准报价 (公开的分煤种出厂含税价) 与 1000 点质量指数之比换算, 合同与煤池用线上保存的那一份。先用 today_blend 出配方; \
用户说某个煤停了/不要, 放进 exclude 重算; 用户给出具体比例时用 check_ratios 验算。";

/// 处理一条消息要用到的线上数据. 只有 tools/call 需要, 由调用方按需加载.
pub struct Context {
    pub master: Value,
    pub storage: UserStorage,
    pub market_price: Option<f64>,
    /// 当前生效的基准报价 (分煤种, 带日期).
    pub benchmarks: Vec<Value>,
    pub rank_k: Option<f64>,
}

/// 这条消息要不要线上数据 (要的话调用方先查库再调 [`handle`]).
pub fn needs_context(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str) == Some("tools/call")
}

/// 处理一条 JSON-RPC 消息. 通知 (没有 id) 返回 None, 调用方不回包.
pub fn handle(message: &Value, context: Option<&Context>) -> Option<Value> {
    let id = message.get("id").filter(|id| !id.is_null())?.clone();
    let outcome = match message.get("method").and_then(Value::as_str) {
        Some("initialize") => Ok(initialize(message)),
        Some("ping") => Ok(json!({})),
        Some("tools/list") => Ok(json!({ "tools": tools() })),
        Some("tools/call") => match context {
            Some(context) => Ok(call_tool(message.get("params"), context)),
            None => Err((-32603, "服务端未加载数据".to_string())),
        },
        Some(other) => Err((-32601, format!("不支持的方法: {other}"))),
        None => Err((-32600, "缺少 method".to_string())),
    };
    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

fn initialize(message: &Value) -> Value {
    let requested = message["params"]["protocolVersion"].as_str().unwrap_or("");
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|v| **v == requested)
        .unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "doudou-blend", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

fn tools() -> Value {
    let quantity = json!({ "type": "number", "description": "采购总吨数; 不填用线上保存的吨数" });
    json!([
        {
            "name": "today_blend",
            "description": "按线上保存的合同与煤池, 全部用估算价求最低成本配方。返回配比、吨数、每种煤估算价、成本、8 项指标体检。",
            "inputSchema": { "type": "object", "properties": {
                "total_quantity": quantity,
                "exclude": { "type": "array", "items": { "type": "string" },
                             "description": "本次不用的煤名 (如停产、拉不出来), 精确煤名" }
            } }
        },
        {
            "name": "check_ratios",
            "description": "按给定配比验算 (不求最优): 算成本与 8 项指标是否达标。",
            "inputSchema": { "type": "object", "required": ["ratios"], "properties": {
                "ratios": { "type": "object", "additionalProperties": { "type": "number" },
                            "description": "煤名 → 份数, 按总和归一 (如 {\"明鑫\": 4, \"豹子沟\": 2})" },
                "total_quantity": quantity
            } }
        },
        {
            "name": "find_coal",
            "description": "按煤名查煤: 状态、煤种、产地、8 项指标 (标出哪些是规格值而非化验值)、质量指数与估算价。",
            "inputSchema": { "type": "object", "required": ["name"], "properties": {
                "name": { "type": "string", "description": "煤名或其中一段" }
            } }
        },
        {
            "name": "get_contract",
            "description": "查看当前合同的 8 项质量要求、采购吨数与估价用的市场价。",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

fn tool_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn call_tool(params: Option<&Value>, context: &Context) -> Value {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let empty = Map::new();
    let args = params
        .and_then(|p| p.get("arguments"))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let outcome = match name {
        "today_blend" => blend(args, context, false),
        "check_ratios" => blend(args, context, true),
        "find_coal" => find_coal(args, context),
        "get_contract" => Ok(contract_text(context)),
        other => Err(format!("没有这个工具: {other}")),
    };
    match outcome {
        Ok(text) => tool_result(text, false),
        Err(message) => tool_result(message, true),
    }
}

fn label(indicator: &str) -> &'static str {
    match indicator {
        "S" => "硫 S",
        "A" => "灰 A",
        "V" => "挥发 V",
        "G" => "粘结 G",
        "Y" => "胶质 Y",
        "petro" => "岩相 σ",
        "CSR" => "焦炭强度 CSR",
        "M" => "水分 M",
        _ => "?",
    }
}

fn bound_text(spec: &Value) -> String {
    let fmt = |v: &Value| v.as_f64().map(|x| format!("{x}")).unwrap_or_default();
    match spec["direction"].as_str() {
        Some("Upper") => format!("≤{}", fmt(&spec["max"])),
        Some("Lower") => format!("≥{}", fmt(&spec["min"])),
        _ => format!("{}~{}", fmt(&spec["min"]), fmt(&spec["max"])),
    }
}

fn blend(args: &Map<String, Value>, context: &Context, fixed: bool) -> Result<String, String> {
    let fixed_ratios = if fixed {
        let ratios = args
            .get("ratios")
            .and_then(Value::as_object)
            .filter(|r| !r.is_empty())
            .ok_or("check_ratios 需要 ratios: 煤名 → 份数")?;
        Some(ratios.clone())
    } else {
        None
    };
    let options = TodayOptions {
        total_quantity: args.get("total_quantity").and_then(Value::as_f64),
        exclude: args
            .get("exclude")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        fixed_ratios,
    };
    let market = Market {
        benchmarks: context.benchmarks.clone(),
        market_price: context.market_price,
    };
    let built = today::build(&context.master, &context.storage, &market, &options)?;
    let body = built.request.to_string();
    let body = crate::admin::inject_rank_interaction(&body, context.rank_k);
    let result: Value = serde_json::from_str(&blend_kit::solve_json(&body))
        .map_err(|e| format!("求解结果解析失败: {e}"))?;
    Ok(format_result(
        &built,
        &result,
        &options,
        context.market_price,
    ))
}

fn format_result(
    built: &today::TodayRequest,
    result: &Value,
    options: &TodayOptions,
    market: Option<f64>,
) -> String {
    let request = &built.request;
    let quantity = request["total_quantity"].as_f64().unwrap_or(0.0);
    let mut out = Vec::new();
    let head = if options.fixed_ratios.is_some() {
        "配比验算"
    } else {
        "今日配方"
    };
    let market_text = match request["price_factor"]["benchmarks"].as_array() {
        Some(list) if !list.is_empty() => {
            let latest = list
                .iter()
                .filter_map(|b| b["date"].as_str())
                .max()
                .unwrap_or("");
            format!("按 {} 条分煤种基准报价估价, 最新 {latest}", list.len())
        }
        _ => result["market_price"]
            .as_f64()
            .or(market)
            .map(|m| format!("标准品 1000 点 = {m:.0} 元/吨"))
            .unwrap_or_else(|| "无市场价".into()),
    };
    out.push(format!(
        "{head} ({}, {quantity:.0} 吨, 全部按估算价, {market_text})",
        built.contract_name
    ));
    if !options.exclude.is_empty() {
        out.push(format!("本次排除: {}", options.exclude.join("、")));
    }

    if result["ok"] != true {
        out.push(format!(
            "无解: {}",
            result["reason"].as_str().unwrap_or("约束冲突")
        ));
        for bound in result["infeasible_bounds"].as_array().into_iter().flatten() {
            out.push(format!("- {}", bound));
        }
    } else {
        if let Some(cost) = result["cost"]["cif_per_ton"].as_f64() {
            out.push(format!(
                "成本 {cost:.1} 元/吨, 合计 {:.1} 万元",
                cost * quantity / 10_000.0
            ));
        }
        // 每种煤的单价: 有报价用报价, 否则用估算价 (记下对标的基准).
        let mut price: Map<String, Value> = Map::new();
        let mut notes: Map<String, Value> = Map::new();
        for value in result["coal_values"].as_array().into_iter().flatten() {
            if let Some(name) = value["name"].as_str() {
                price.insert(name.into(), value["fair"].clone());
                notes.insert(name.into(), Value::String(price_note(value)));
            }
        }
        for coal in request["coals"].as_array().into_iter().flatten() {
            if let Some(name) = coal["name"].as_str() {
                price.insert(name.into(), coal["fob"].clone());
            }
        }
        let types = coal_types(request);
        // 按整铲配时每种煤配几铲 (orders[].shovels).
        let shovels: Map<String, Value> = result["orders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|order| {
                Some((
                    order["coal"].as_str()?.to_string(),
                    order["shovels"].clone(),
                ))
            })
            .collect();
        out.push("配方:".into());
        for (name, ratio) in sorted_recipe(&result["recipe"]) {
            let unit = price
                .get(name.as_str())
                .and_then(Value::as_f64)
                .map(|p| format!(", 估算出厂价 {p:.0}"))
                .unwrap_or_default();
            let note = notes.get(name.as_str()).and_then(Value::as_str);
            let unit = format!("{unit}{}", note.unwrap_or_default());
            let shovel = shovels
                .get(name.as_str())
                .and_then(Value::as_u64)
                .map(|count| format!(" · {count} 铲"))
                .unwrap_or_default();
            out.push(format!(
                "- {} {:.1}% ({:.0} 吨{shovel}{unit})",
                typed(&name, &types),
                ratio * 100.0,
                ratio * quantity
            ));
        }
    }

    let specs: Vec<&Value> = request["specs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["enabled"] != false)
        .collect();
    let checks = result["indicator_check"].as_array();
    if checks.is_some_and(|c| !c.is_empty()) {
        out.push("指标:".into());
        for check in checks.into_iter().flatten() {
            let indicator = check["indicator"].as_str().unwrap_or("");
            let contract = specs
                .iter()
                .find(|s| s["indicator"] == indicator)
                .map(|s| format!(" (合同 {})", bound_text(s)))
                .unwrap_or_default();
            let status = match check["status"].as_str() {
                Some("Fail") => "不达标",
                Some("Pass") | Some("TolerancePass") => "达标",
                _ => "估算",
            };
            let tight = if check["binding"] == true {
                "·卡线"
            } else {
                ""
            };
            out.push(format!(
                "- {} {:.2}{contract} {status}{tight}",
                label(indicator),
                check["value"].as_f64().unwrap_or(f64::NAN)
            ));
        }
    }
    if let Some(csr) = result["csr_stamp_estimate"].as_f64() {
        out.push(format!("捣固炼焦 CSR 估算 {csr:.1}"));
    }
    let notes: Vec<String> = built
        .notes
        .iter()
        .cloned()
        .chain(
            result["warnings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from),
        )
        .collect();
    if !notes.is_empty() {
        out.push("提示:".into());
        out.extend(notes.into_iter().map(|n| format!("- {n}")));
    }
    out.join("\n")
}

/// 煤名 → 煤种. 进 LP 的煤和按估算价求解的煤都带.
fn coal_types(request: &Value) -> Map<String, Value> {
    let lp = request["coals"].as_array().into_iter().flatten();
    let extra = request["price_factor"]["extra_coals"]
        .as_array()
        .into_iter()
        .flatten();
    lp.chain(extra)
        .filter_map(|coal| {
            let name = coal["name"].as_str()?;
            Some((name.to_string(), coal.get("coal_type")?.clone()))
        })
        .collect()
}

/// "明鑫 (瘦煤)"; 没标煤种就只写煤名.
fn typed(name: &str, types: &Map<String, Value>) -> String {
    match types.get(name).and_then(Value::as_str) {
        Some(kind) => format!("{name} ({kind})"),
        None => name.to_string(),
    }
}

/// 配方按占比降序, 去掉可忽略的零头.
fn sorted_recipe(recipe: &Value) -> Vec<(String, f64)> {
    let mut list: Vec<(String, f64)> = recipe
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, ratio)| ratio.as_f64().map(|r| (name.clone(), r)))
        .filter(|(_, r)| *r > 1e-4)
        .collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    list
}

fn find_coal(args: &Map<String, Value>, context: &Context) -> Result<String, String> {
    let query = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or("find_coal 需要 name")?
        .to_lowercase();
    let master_coals = context.master["coals"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let user_coals = context
        .storage
        .user_coals
        .as_array()
        .cloned()
        .unwrap_or_default();
    let matches: Vec<Value> = user_coals
        .into_iter()
        .chain(master_coals)
        .filter(|c| {
            c["name"]
                .as_str()
                .is_some_and(|n| n.to_lowercase().contains(&query))
        })
        .take(10)
        .collect();
    if matches.is_empty() {
        return Err(format!("煤库里没有名字含「{query}」的煤"));
    }

    let factor: Option<blend_kit::PriceFactor> = context
        .master
        .get("price_factor")
        .and_then(|f| serde_json::from_value(f.clone()).ok())
        .map(|mut f: blend_kit::PriceFactor| {
            f.benchmarks = context
                .benchmarks
                .iter()
                .filter_map(|b| serde_json::from_value(b.clone()).ok())
                .collect();
            if f.benchmarks.is_empty() {
                f.market_price = context.market_price.or(f.market_price);
            }
            f
        });
    let mut out = Vec::new();
    for coal in matches {
        let name = coal["name"].as_str().unwrap_or("");
        let status = match coal["status"].as_str() {
            Some("verified") | Some("active") => "在用",
            Some("archived") => "已停用",
            Some("draft") => "待核实",
            Some("incomplete") => "数据不全",
            _ => "未知",
        };
        let spec_fields: Vec<&str> = coal["confidence"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, level)| level.as_str() == Some("spec"))
            .map(|(k, _)| k.as_str())
            .collect();
        let props: Vec<String> = today::INDICATORS
            .iter()
            .filter_map(|k| {
                coal["props"][*k].as_f64().map(|v| {
                    let mark = if spec_fields.contains(k) {
                        "(规格)"
                    } else {
                        ""
                    };
                    format!("{} {v}{mark}", label(k))
                })
            })
            .collect();
        let tags: Vec<&str> = ["coal_type", "form", "region"]
            .iter()
            .filter_map(|key| coal[*key].as_str())
            .collect();
        let mut line = format!("{name} [{status}] {}", tags.join(" "));
        line.push_str(&format!("\n  指标: {}", props.join(", ")));
        if let Some(factor) = &factor {
            // 带上煤种、形态、地区: 估价按它们逐层对标基准, 与 today_blend 同一口径.
            let as_coal: Result<blend_kit::Coal, _> = serde_json::from_value(json!({
                "name": name, "props": coal["props"], "fob": 0.0, "frt": 0.0,
                "coal_type": coal["coal_type"], "form": coal["form"], "region": coal["region"]
            }));
            if let Ok(estimate) = as_coal
                .map_err(|_| ())
                .and_then(|c| blend_kit::price_factor::evaluate(&[c], factor).map_err(|_| ()))
            {
                if let Some(value) = estimate.coal_values.first() {
                    let price = estimate
                        .market
                        .map(|_| {
                            let note = serde_json::to_value(value)
                                .map(|v| price_note(&v))
                                .unwrap_or_default();
                            format!(", 估算出厂价 {:.0} 元/吨{note}", value.fair)
                        })
                        .unwrap_or_default();
                    line.push_str(&format!("\n  质量指数 {:.0}{price}", value.points));
                }
            }
        }
        if let Some(note) = coal["note"].as_str() {
            line.push_str(&format!("\n  备注: {note}"));
        }
        out.push(line);
    }
    if !spec_fields_legend_needed(&out) {
        return Ok(out.join("\n"));
    }
    out.push("(规格) = 合同/报价的保证界限, 不是化验值".into());
    Ok(out.join("\n"))
}

/// 估算价的出处说明 (一条 coal_values): ", 对标X"; 没对标基准为空.
fn price_note(value: &Value) -> String {
    value["benchmark"]
        .as_str()
        .map(|benchmark| format!(", 对标{benchmark}"))
        .unwrap_or_default()
}

fn spec_fields_legend_needed(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("(规格)"))
}

fn contract_text(context: &Context) -> String {
    let (specs, name) = match context.storage.contract.as_ref().filter(|c| c.is_array()) {
        Some(specs) => (specs.clone(), "用户自定义合同".to_string()),
        None => (
            context.master["default_contract"]["specs"].clone(),
            context.master["default_contract"]["name"]
                .as_str()
                .unwrap_or("默认合同")
                .to_string(),
        ),
    };
    let mut out = vec![format!("{name}, 采购 {:.0} 吨", context.storage.quantity)];
    for spec in specs.as_array().into_iter().flatten() {
        if spec["enabled"] == false {
            continue;
        }
        out.push(format!(
            "- {} {}",
            label(spec["indicator"].as_str().unwrap_or("")),
            bound_text(spec)
        ));
    }
    if context.benchmarks.is_empty() {
        out.push(match context.market_price {
            Some(m) => format!(
                "估价市场价: 标准品 1000 点 = {m:.0} (焦煤现货指数点位, 无基准报价时的退路)"
            ),
            None => "估价市场价: 没有基准报价, 现货指数也取不到".into(),
        });
    } else {
        out.push("估价基准报价 (同煤种、指标最接近的一条换算):".into());
        for benchmark in &context.benchmarks {
            out.push(benchmark_line(benchmark));
        }
    }
    out.join("\n")
}

/// "- 焦煤 晋中中硫主焦 S1.3 A10.5 G80: 2050 (2026-10-10)", 按指数推算过的再注明.
fn benchmark_line(benchmark: &Value) -> String {
    let props = &benchmark["props"];
    let spec: Vec<String> = ["S", "A", "V", "G", "Y"]
        .into_iter()
        .filter_map(|key| props[key].as_f64().map(|v| format!("{key}{v}")))
        .collect();
    let price = benchmark["price"].as_f64().unwrap_or(0.0);
    let drift = match benchmark["current_price"].as_f64() {
        Some(now) => format!(", 按指数推算到今天 {now:.0}"),
        None => String::new(),
    };
    format!(
        "- {} {} {}: {price:.0} ({}{drift})",
        benchmark["coal_type"].as_str().unwrap_or("未分煤种"),
        benchmark["name"].as_str().unwrap_or(""),
        spec.join(" "),
        benchmark["date"].as_str().unwrap_or("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context {
            master: json!({
                "default_contract": {"name": "默认合同", "specs": [
                    {"indicator": "S", "direction": "Upper", "max": 1.5, "enabled": true}
                ]},
                "price_factor": {"description": "x", "adjusts": [
                    {"indicator": "S", "direction": "Upper", "base": 1.3, "premium_rate": 75, "discount_rate": 125}
                ]},
                "coals": [
                    {"name": "低硫", "status": "active", "coal_type": "焦煤", "props": {"S": 0.6}, "confidence": {"S": "spec"}},
                    {"name": "高硫", "status": "active", "coal_type": "瘦煤", "props": {"S": 2.4}},
                    {"name": "停产", "status": "archived", "props": {"S": 0.1}}
                ]
            }),
            storage: UserStorage {
                initialized: true,
                coal_prefs: json!({"低硫": {"enabled": true}, "高硫": {"enabled": true}, "停产": {"enabled": true}}),
                contract: None,
                quantity: 1000.0,
                user_coals: json!([]),
                shovel_tons: None,
            },
            market_price: Some(2000.0),
            benchmarks: Vec::new(),
            rank_k: None,
        }
    }

    fn call(name: &str, arguments: Value) -> (String, bool) {
        call_with(&context(), name, arguments)
    }

    fn call_with(context: &Context, name: &str, arguments: Value) -> (String, bool) {
        let message = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                             "params": {"name": name, "arguments": arguments}});
        let response = handle(&message, Some(context)).unwrap();
        let result = &response["result"];
        (
            result["content"][0]["text"].as_str().unwrap().to_string(),
            result["isError"] == true,
        )
    }

    #[test]
    fn test_initialize_and_list() {
        let init = handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-03-26"}}),
            None,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        let unknown = handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "1999-01-01"}}),
            None,
        )
        .unwrap();
        assert_eq!(unknown["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);

        let list = handle(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            None,
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["today_blend", "check_ratios", "find_coal", "get_contract"]
        );
    }

    #[test]
    fn test_notification_gets_no_reply_and_unknown_method_errors() {
        assert!(handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            None
        )
        .is_none());
        let reply = handle(
            &json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
            None,
        )
        .unwrap();
        assert_eq!(reply["error"]["code"], -32601);
        assert!(needs_context(&json!({"method": "tools/call"})));
        assert!(!needs_context(&json!({"method": "tools/list"})));
    }

    #[test]
    fn test_today_blend_uses_estimates_and_skips_archived() {
        let (text, is_error) = call("today_blend", json!({}));
        assert!(!is_error, "{text}");
        assert!(text.contains("今日配方"), "{text}");
        assert!(text.contains("低硫"), "{text}");
        assert!(!text.contains("停产"), "已停用的煤不得进配方: {text}");
        assert!(text.contains("标准品 1000 点 = 2000"), "{text}");
        assert!(text.contains("硫 S"), "{text}");
        // 配方行带煤种 (硫按一位小数截断判, 卡在 1.6 以下). 今日页已不显示估价可靠度, 这里同样不带.
        // 一铲 3 吨, 采购 1000 吨 = 333 铲, 整铲配. 连续解低硫 44.4% 卡在硫线上, 凑成 148 铲
        // 硫就过线, 所以挪一铲给低硫: 149 铲 = 44.7%.
        assert!(
            text.contains("- 低硫 (焦煤) 44.7% (447 吨 · 149 铲"),
            "{text}"
        );
        assert!(!text.contains("估价可靠度"), "{text}");
    }

    #[test]
    fn test_exclude_and_check_ratios() {
        let (text, _) = call("today_blend", json!({"exclude": ["低硫"]}));
        assert!(text.contains("本次排除: 低硫"), "{text}");
        assert!(text.contains("无解"), "只剩高硫煤时硫超标应无解: {text}");

        // 1:2 → 硫 (0.6 + 2×2.4) / 3 = 1.8 > 1.5
        let (text, is_error) = call("check_ratios", json!({"ratios": {"低硫": 1, "高硫": 2}}));
        assert!(!is_error, "{text}");
        assert!(
            text.contains("配比验算") && text.contains("不达标"),
            "{text}"
        );

        let (text, is_error) = call("check_ratios", json!({}));
        assert!(is_error && text.contains("ratios"), "{text}");
    }

    #[test]
    fn test_find_coal_and_contract() {
        let (text, is_error) = call("find_coal", json!({"name": "低"}));
        assert!(!is_error, "{text}");
        assert!(
            text.contains("低硫 [在用]") && text.contains("(规格)") && text.contains("估算出厂价"),
            "{text}"
        );
        let (text, is_error) = call("find_coal", json!({"name": "没有这煤"}));
        assert!(is_error, "{text}");

        let (text, _) = call("get_contract", json!({}));
        assert!(
            text.contains("硫 S ≤1.5") && text.contains("1000 吨"),
            "{text}"
        );
    }

    #[test]
    fn test_benchmarks_drive_estimates_in_every_tool() {
        let mut context = context();
        context.market_price = None;
        context.benchmarks = vec![
            json!({"name": "晋中中硫主焦", "coal_type": "焦煤", "props": {"S": 1.3},
                   "price": 2050.0, "date": "2026-10-10"}),
            json!({"name": "吕梁高硫瘦煤", "coal_type": "瘦煤", "props": {"S": 2.8},
                   "price": 1860.0, "date": "2026-10-09", "current_price": 1840.0}),
        ];
        let (text, is_error) = call_with(&context, "today_blend", json!({}));
        assert!(!is_error, "{text}");
        assert!(
            text.contains("按 2 条分煤种基准报价估价, 最新 2026-10-10"),
            "{text}"
        );
        assert!(
            text.contains("对标晋中中硫主焦") && text.contains("对标吕梁高硫瘦煤"),
            "{text}"
        );

        let (text, _) = call_with(&context, "find_coal", json!({"name": "高硫"}));
        assert!(text.contains("估算出厂价"), "{text}");

        let (text, _) = call_with(&context, "get_contract", json!({}));
        assert!(
            text.contains("- 焦煤 晋中中硫主焦 S1.3: 2050 (2026-10-10)"),
            "{text}"
        );
        assert!(text.contains("按指数推算到今天 1840"), "{text}");
    }

    #[test]
    fn test_find_coal_estimates_by_type_and_form() {
        let mut context = context();
        context.market_price = None;
        context.master["coals"][1]["form"] = json!("筛选精煤");
        context.master["coals"][1]["region"] = json!("山西临汾");
        // 前两条与"高硫"硫分相同, 但一条是焦煤、一条是浮精; 它是瘦煤筛精, 只能对标瘦煤筛精.
        context.benchmarks = vec![
            json!({"name": "高硫焦", "coal_type": "焦煤", "form": "筛选精煤", "props": {"S": 2.4},
                   "price": 2000.0, "date": "2026-10-10"}),
            json!({"name": "高硫瘦浮精", "coal_type": "瘦煤", "form": "浮选精煤", "props": {"S": 2.4},
                   "price": 1700.0, "date": "2026-10-10"}),
            json!({"name": "中硫瘦筛精", "coal_type": "瘦煤", "form": "筛选精煤", "props": {"S": 1.3},
                   "price": 1800.0, "date": "2026-10-10"}),
        ];
        let (text, _) = call_with(&context, "find_coal", json!({"name": "高硫"}));
        assert!(
            text.contains("高硫 [在用] 瘦煤 筛选精煤 山西临汾"),
            "{text}"
        );
        assert!(text.contains("对标中硫瘦筛精"), "{text}");
    }
}
