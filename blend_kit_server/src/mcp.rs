//! MCP (Model Context Protocol) 入口: 让手机上的 Agent (Kimi、Claude 等) 直接调配煤.
//!
//! 传输是无状态的 Streamable HTTP: 每个 POST 带一条 (或一批) JSON-RPC 消息, 直接回 JSON,
//! 不开 SSE 流, 不发会话 id —— 四个工具都是一问一答, 用不着服务端推送.
//!
//! 求解请求一律由 [`crate::today`] 按线上保存的合同与煤池组装, Agent 不必理解请求格式,
//! 算出来的与网页今日屏同一口径。本模块只做协议分派与结果排版, 不碰库也不联网 ——
//! 数据由调用方装进 [`Context`] 再传进来, 所以全部可以用固定数据单测。
//!
//! 排版给聊天模型读, 每个数都要自带含义: 判定规则、估算与否、哪个数对合同负责都写成
//! 人话, 不吐原始 JSON —— 模型拿不准时会自己编一个解释转告用户.
use blend_kit::{
    AcceptanceMode, AcceptanceRule, Direction, EvaluationMethod, EvaluationStatus, IndicatorCheck,
    InfeasibleBound,
};
use serde_json::{json, Map, Value};

use crate::database::UserStorage;
use crate::today::{self, Market, TodayOptions};

/// 支持的协议版本, 新的在前. 客户端要的版本在列表里就照回, 否则回最新的.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "豆哥配煤: 炼焦配煤的最低成本配方。所有煤价都是估算价: 按指标 (含挥发分) 最接近的几条 \
基准报价 (公开的出厂含税价与指数) 加权, 用 1000 点质量指数之比换算, 合同与煤池用线上保存的那一份。先用 today_blend 出配方; \
用户说某个煤停了/不要, 把完整煤名放进 exclude 重算 (一个词对应几种煤时逐个列出, 拿不准先用 find_coal 查); \
用户给出具体比例时用 check_ratios 验算。合同每项有判定规则 (如保留 1 位小数截断后再比), 指标行里的达标结论已按规则判过, \
照转给用户即可, 不要按原值自行改判。";

/// 判原值是否越过合同数时容的浮点残差, 与核心 `quality` 的 `CHECK_TOLERANCE` 一致.
const RAW_TOLERANCE: f64 = 1e-8;

/// 核心在凑不成整铲时给的警告前缀 (`blend_kit::shovel`).
const SHOVEL_WARNING: &str = "凑不成整铲";

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
            "description": "按线上保存的合同与煤池, 全部用估算价求最低成本配方。返回配比、吨数、每种煤估算价、成本、8 项指标体检 (已按合同判定规则判过达标与否)。",
            "inputSchema": { "type": "object", "properties": {
                "total_quantity": quantity,
                "exclude": { "type": "array", "items": { "type": "string" },
                             "description": "本次不用的煤 (如停产、拉不出来), 每项必须是完整煤名。用户说的一个词对应几种煤时 (如「东山」对应东山一号、东山二号), 要用哪几种就逐个列出完整煤名; 拿不准先用 find_coal 查。有一个名字对不上就不求解, 并列出名字相近的煤" }
            } }
        },
        {
            "name": "check_ratios",
            "description": "按给定配比验算 (不求最优): 算成本与 8 项指标是否达标。",
            "inputSchema": { "type": "object", "required": ["ratios"], "properties": {
                "ratios": { "type": "object", "additionalProperties": { "type": "number" },
                            "description": "完整煤名 → 份数, 按总和归一 (如 {\"明鑫\": 4, \"豹子沟\": 2}); 只能用参与今日求解的煤, 点到不参与的煤会说明原因; 煤名对不上会列出名字相近、能参与求解的煤" },
                "total_quantity": quantity
            } }
        },
        {
            "name": "find_coal",
            "description": "按煤名查煤: 状态、是否参与今日求解、煤种、产地、8 项指标 (按用户在煤池改过的值, 标出哪些是用户改过的、哪些是规格值而非化验值)、质量指数与估算价。",
            "inputSchema": { "type": "object", "required": ["name"], "properties": {
                "name": { "type": "string", "description": "煤名或其中一段" }
            } }
        },
        {
            "name": "get_contract",
            "description": "查看当前合同的 8 项质量要求 (含每项的判定规则)、采购吨数与估价用的市场价。",
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

/// 保留 `digits` 位小数后去掉末尾的 0: 1.20 → "1.2", 2.00 → "2".
fn trim_num(x: f64, digits: usize) -> String {
    let text = format!("{x:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// 两位小数; 与 `other` 显示成同一个数而实际不同时多给两位, 免得"要求 ≤10, 最好做到 10".
fn trim_num_apart(x: f64, other: f64) -> String {
    let short = trim_num(x, 2);
    if short == trim_num(other, 2) && (x - other).abs() > 1e-9 {
        trim_num(x, 4)
    } else {
        short
    }
}

/// 本项的合同判定规则. 合同没写时照核心 `quality::acceptance_rule` 的旧口径:
/// 今日请求打开了 `truncate_decimal`, 岩相以外一律保留 1 位小数截断.
fn acceptance(spec: &Value) -> AcceptanceRule {
    spec.get("acceptance")
        .filter(|rule| !rule.is_null())
        .and_then(|rule| serde_json::from_value(rule.clone()).ok())
        .unwrap_or_else(|| {
            if today::TRUNCATE_DECIMAL && spec["indicator"] != "petro" {
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

fn rule_decimals(rule: &AcceptanceRule) -> usize {
    usize::from(rule.decimals.unwrap_or(0).min(6))
}

/// "保留 1 位小数、截断后判定" 这类短说法.
fn rule_head(rule: &AcceptanceRule) -> String {
    let decimals = rule_decimals(rule);
    let how = match rule.mode {
        AcceptanceMode::Raw => return "按原值判定, 不取舍".into(),
        AcceptanceMode::Truncate => "截断",
        AcceptanceMode::Round => "四舍五入",
    };
    if decimals == 0 {
        format!("{how}取整后判定")
    } else {
        format!("保留 {decimals} 位小数、{how}后判定")
    }
}

/// 判定规则的完整说明, 截断/四舍五入时带一个贴着合同界的例子 ("1.09 记作 1.0").
fn rule_text(spec: &Value, rule: &AcceptanceRule) -> String {
    let lower = spec["direction"] == "Lower";
    let bound = if lower {
        spec["min"].as_f64()
    } else {
        spec["max"].as_f64()
    };
    let mut text = rule_head(rule);
    if let Some(clause) = tolerance_clause(spec, rule) {
        text.push_str(&format!("; {clause}"));
    } else if let Some(bound) = bound {
        if let Some((shown, judged)) = rule_example(bound, rule, lower) {
            text.push_str(&format!(": {shown} 记作 {judged}"));
            // 下限的例子容易看反, 把结论也说出来.
            if lower {
                let enough = judged.parse::<f64>().is_ok_and(|j| j >= bound - 1e-9);
                let verdict = if enough { "够" } else { "不够" };
                text.push_str(&format!(", 所以{verdict} {}", bound_text(spec)));
            }
        }
    }
    text
}

/// 合同另允许的偏差: "合同另允许超出上限 0.5 以内"; 没有偏差为 None.
fn tolerance_clause(spec: &Value, rule: &AcceptanceRule) -> Option<String> {
    if rule.tolerance <= 0.0 {
        return None;
    }
    let side = match spec["direction"].as_str() {
        Some("Upper") => "超出上限",
        Some("Lower") => "低于下限",
        _ => "超出上下限",
    };
    Some(format!(
        "合同另允许{side} {} 以内",
        trim_num(rule.tolerance, 6)
    ))
}

/// 贴着合同界的取舍例子, 只为说明规则: 上限给落在界内的那一格, 下限给容易误判的那一格.
fn rule_example(bound: f64, rule: &AcceptanceRule, lower: bool) -> Option<(String, String)> {
    let decimals = rule_decimals(rule);
    let scale = 10_f64.powi(decimals as i32);
    let quantum = 1.0 / scale;
    let scaled = bound * scale;
    let scaled = if (scaled - scaled.round()).abs() < 1e-9 {
        scaled.round()
    } else {
        scaled
    };
    let (base, offset) = match (rule.mode, lower) {
        (AcceptanceMode::Raw, _) => return None,
        // 1.09 截断记 1.0: 合同 ≤1 认.
        (AcceptanceMode::Truncate, false) => (scaled.floor() / scale, 0.9 * quantum),
        // 67.99 截断记 67.9: 合同 ≥68 不认.
        (AcceptanceMode::Truncate, true) => ((scaled.ceil() - 1.0) / scale, 0.9 * quantum),
        // 1.04 四舍五入记 1.0.
        (AcceptanceMode::Round, false) => (scaled.floor() / scale, 0.4 * quantum),
        // 67.95 四舍五入记 68.0.
        (AcceptanceMode::Round, true) => (scaled.ceil() / scale, -0.5 * quantum),
    };
    Some((
        format!("{:.*}", decimals + 1, base + offset),
        format!("{base:.decimals$}"),
    ))
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
    Ok(format_result(&built, &result, context.market_price))
}

fn format_result(built: &today::TodayRequest, result: &Value, market: Option<f64>) -> String {
    let request = &built.request;
    let quantity = request["total_quantity"].as_f64().unwrap_or(0.0);
    let mut out = Vec::new();
    let head = if request.get("fixed_ratios").is_some() {
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
    if !built.excluded.is_empty() {
        out.push(format!("本次排除: {}", built.excluded.join("、")));
    }
    if !built.excluded_idle.is_empty() {
        let idle: Vec<String> = built
            .excluded_idle
            .iter()
            .map(|(name, reason)| format!("{name} ({reason})"))
            .collect();
        out.push(format!("要排除的煤本来就不参与求解: {}", idle.join("、")));
    }

    let specs: Vec<&Value> = request["specs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["enabled"] != false)
        .collect();
    let spec_of = |indicator: &str| specs.iter().copied().find(|s| s["indicator"] == indicator);
    let (shovel_warnings, mut warnings): (Vec<String>, Vec<String>) = result["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .partition(|w| w.starts_with(SHOVEL_WARNING));

    if result["ok"] != true {
        let reason = result["reason"].as_str().unwrap_or("约束冲突");
        // 求解器没算出结论、也没证实不可行: 不能说"无解", 合同也许本来配得出.
        if result["undetermined"] == true {
            out.push(format!(
                "这次没算出结论: {reason}。这不等于合同配不出来, 可以再算一次"
            ));
        } else {
            out.push(format!("无解: {reason}"));
        }
        let bounds: Vec<InfeasibleBound> =
            serde_json::from_value(result["infeasible_bounds"].clone()).unwrap_or_default();
        for bound in &bounds {
            out.push(format!(
                "- {}",
                infeasible_line(bound, spec_of(&bound.indicator))
            ));
        }
        warnings.extend(shovel_warnings);
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
        // 凑整失败的配方照样列出来, 但不放在末尾提示里: 模型会忽略它, 照未取整的吨数去凑铲.
        for warning in &shovel_warnings {
            out.push(format!(
                "注意: {warning}。下面的吨数没有按整铲取整, 不能直接照此装车"
            ));
        }
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

    let checks: Vec<IndicatorCheck> =
        serde_json::from_value(result["indicator_check"].clone()).unwrap_or_default();
    if !checks.is_empty() {
        out.push("指标 (达标结论已按合同判定规则判过):".into());
        for check in &checks {
            out.push(indicator_line(check, spec_of(&check.indicator)));
        }
    }
    if let Some(stamp) = result["csr_stamp_estimate"].as_f64() {
        let csr = checks.iter().find(|c| c.indicator == "CSR");
        out.push(stamp_line(
            stamp,
            spec_of("CSR"),
            csr,
            result["csr_interaction_penalty"].as_f64(),
        ));
    }
    let notes: Vec<String> = built.notes.iter().cloned().chain(warnings).collect();
    if !notes.is_empty() {
        out.push("提示:".into());
        out.extend(notes.into_iter().map(|n| format!("- {n}")));
    }
    out.join("\n")
}

/// 无解诊断的一行人话. `relax_to` 是"合同上要填的数" (计价项是拒收线), 核心已按判定规则
/// 与允许偏差逐档真解过, 所以可能比 `achievable` 还松一侧 —— 不说清这一点, 模型会以为
/// 两个数互相矛盾.
fn infeasible_line(bound: &InfeasibleBound, spec: Option<&Value>) -> String {
    let sign = match bound.direction {
        Direction::Upper => "≤",
        Direction::Lower => "≥",
        Direction::Range => "",
    };
    let priced = spec.is_some_and(|s| s["enforcement"] == "Priced");
    let what = if priced { "拒收线" } else { "合同要求" };
    let achievable = trim_num_apart(bound.achievable, bound.required);
    let mut line = format!(
        "{}: {what} {sign}{}, 其余要求都满足时这批煤最好只能做到 {achievable}",
        label(&bound.indicator),
        trim_num(bound.required, 4),
    );
    match bound.relax_to {
        Some(relax) => {
            let relax_text = format!("{sign}{}", trim_num(relax, 4));
            let filled = if priced {
                line.push_str(&format!("; 拒收线放宽到 {relax_text} 才配得出"));
                "这是拒收线要填的数"
            } else {
                line.push_str(&format!("; 合同上这一项写成 {relax_text} 才配得出"));
                "这是合同上要填的数"
            };
            let past = match bound.direction {
                Direction::Upper => relax < bound.achievable,
                Direction::Lower => relax > bound.achievable,
                Direction::Range => false,
            };
            let rule = spec.map(acceptance).unwrap_or_default();
            let tolerance = spec.and_then(|s| tolerance_clause(s, &rule));
            if past {
                let side = if bound.direction == Direction::Upper {
                    "小"
                } else {
                    "大"
                };
                let looser = format!("所以比 {achievable} 还{side}也配得出");
                line.push_str(&match (rule.mode, tolerance) {
                    (AcceptanceMode::Raw, Some(clause)) => {
                        format!(" ({filled}: 本项按原值判定, 但{clause}, {looser})")
                    }
                    (_, Some(clause)) => format!(
                        " ({filled}, 已按本项判定规则「{}」并计入{clause}逐档真解过, {looser})",
                        rule_head(&rule)
                    ),
                    (_, None) => format!(
                        " ({filled}, 已按本项判定规则「{}」逐档真解过, {looser})",
                        rule_head(&rule)
                    ),
                });
            } else {
                line.push_str(&format!(" ({filled}, 已按本项判定规则逐档真解过)"));
            }
        }
        None => line.push_str("; 只放宽这一项还配不出来, 要同时调整别的要求或煤池"),
    }
    if bound.margin > 0.0 {
        let margin = trim_num(bound.margin, 4);
        line.push_str(&format!(
            "; 本项设了安全余量 {margin}, 系统按比合同严 {margin} 的线执行"
        ));
    }
    line
}

/// 指标行: 真实值, 合同规则取舍后的判定值 (与真实值不同时), 合同界, 结论.
fn indicator_line(check: &IndicatorCheck, spec: Option<&Value>) -> String {
    let rule = spec.map(acceptance);
    let value = if check.method == EvaluationMethod::Unavailable {
        check.proxy_value
    } else {
        Some(check.value)
    };
    let mut line = format!("- {}", label(&check.indicator));
    if let Some(value) = value {
        let shown = display_value(value, check.judged_value, spec, rule.as_ref());
        line.push_str(&format!(" {shown}"));
        if let (Some(rule), Some(judged)) = (&rule, check.judged_value) {
            if rule.mode != AcceptanceMode::Raw && (judged - value).abs() > 1e-9 {
                line.push_str(&format!(", 按合同规则记 {judged:.*}", rule_decimals(rule)));
            }
        }
    }
    if let Some(spec) = spec {
        line.push_str(&format!(" (合同 {})", bound_text(spec)));
    }
    line.push_str(&format!(": {}", status_text(check, spec)));
    if check.binding {
        line.push_str("·卡线");
    }
    line
}

/// 真实值的显示: 至少 2 位小数, 四舍五入后加位到"照显示的数按合同规则判, 与核心的判定
/// 一致"为止, 免得一次显示 10.10 达标、一次显示 10.10 不达标. 加到 6 位仍不一致 (原值离
/// 格线只差求解残差) 时不再四舍五入: 先朝判定那一侧截断; 核心按放行量把残差判进另一格时,
/// 取那一格里离原值最近的 6 位小数, 免得出现"8.000000, 按合同规则记 7.9".
fn display_value(
    value: f64,
    judged: Option<f64>,
    spec: Option<&Value>,
    rule: Option<&AcceptanceRule>,
) -> String {
    let (Some(spec), Some(rule)) = (spec, rule) else {
        return format!("{value:.2}");
    };
    let consistent = |shown: f64| match rule.mode {
        AcceptanceMode::Raw => {
            within(shown, spec, rule.tolerance) == within(value, spec, rule.tolerance)
        }
        AcceptanceMode::Truncate | AcceptanceMode::Round => {
            judged.is_none_or(|j| (quantize(shown, rule) - j).abs() < 1e-9)
        }
    };
    let start = rule_decimals(rule).max(2);
    let last = start + 4;
    for digits in start..=last {
        let text = format!("{value:.digits$}");
        if consistent(text.parse().unwrap_or(value)) {
            return text;
        }
    }
    let scale = 10_f64.powi(last as i32);
    let scaled = value * scale;
    let scaled = if (scaled - scaled.round()).abs() < 1e-9 {
        scaled.round()
    } else {
        scaled
    };
    for shown in [scaled.floor() / scale, scaled.ceil() / scale] {
        if consistent(shown) {
            return format!("{shown:.last$}");
        }
    }
    if let Some(judged) = judged {
        let quantum = 1.0 / 10_f64.powi(rule_decimals(rule) as i32);
        let unit = 1.0 / scale;
        let cell = match rule.mode {
            AcceptanceMode::Raw => None,
            AcceptanceMode::Truncate => Some((judged, judged + quantum - unit)),
            AcceptanceMode::Round => Some((judged - quantum / 2.0, judged + quantum / 2.0 - unit)),
        };
        if let Some((low, high)) = cell {
            return format!("{:.last$}", value.clamp(low, high));
        }
    }
    format!("{value:.last$}")
}

/// 只用来挑显示位数: 显示出来的数按规则取舍后是多少. 判定值本身一律用核心给的.
fn quantize(shown: f64, rule: &AcceptanceRule) -> f64 {
    let scale = 10_f64.powi(rule_decimals(rule) as i32);
    let scaled = shown * scale;
    // 显示值是有限位小数, 乘出来的浮点噪声先收掉, 否则 4.35×100 会截成 434.
    let scaled = if (scaled - scaled.round()).abs() < 1e-6 {
        scaled.round()
    } else {
        scaled
    };
    match rule.mode {
        AcceptanceMode::Raw => shown,
        AcceptanceMode::Truncate => scaled.trunc() / scale,
        AcceptanceMode::Round => scaled.round() / scale,
    }
}

fn within(x: f64, spec: &Value, tolerance: f64) -> bool {
    let near = |b: f64| 1e-9 * (1.0 + b.abs());
    let upper = spec["max"]
        .as_f64()
        .filter(|_| spec["direction"] != "Lower")
        .is_none_or(|max| x <= max + tolerance + near(max));
    let lower = spec["min"]
        .as_f64()
        .filter(|_| spec["direction"] != "Upper")
        .is_none_or(|min| x >= min - tolerance - near(min));
    upper && lower
}

/// 核心状态逐个翻成一眼能看出过没过的说法.
fn status_text(check: &IndicatorCheck, spec: Option<&Value>) -> String {
    match check.status {
        EvaluationStatus::Fail => "不达标".into(),
        EvaluationStatus::Pass => if spec.is_some() {
            "达标"
        } else {
            "合同无要求"
        }
        .into(),
        // 原值越过合同数, 但按合同判定规则 (取舍、允许偏差) 或计价条款仍放行.
        EvaluationStatus::TolerancePass => {
            if spec.is_some_and(|s| s["enforcement"] == "Priced") {
                priced_text(check)
            } else {
                "达标 (原值超出合同数, 按合同判定规则算达标)".into()
            }
        }
        // 判定规则下在界内, 但值是估算 (线性代理未经实测校准); 核心对这类指标先报"未验证",
        // 不再区分原值是否越过合同数, 所以这里自己看一眼原值.
        // 或有参配煤缺这项数据, 根本判不了.
        EvaluationStatus::Unverified => match spec {
            _ if check.method == EvaluationMethod::Unavailable => {
                "无法判定 (有参配煤缺这项数据)".into()
            }
            None => "合同无要求 (估算值)".into(),
            Some(spec) if within(check.value, spec, RAW_TOLERANCE) => "界内 (估算值)".into(),
            Some(spec) if spec["enforcement"] == "Priced" => {
                format!("{} (估算值)", priced_text(check))
            }
            Some(_) => "界内 (估算值; 原值超出合同数, 按合同判定规则算界内)".into(),
        },
    }
}

/// 计价项超了合同界、没到拒收线.
fn priced_text(check: &IndicatorCheck) -> String {
    match check.penalty_per_ton.filter(|p| *p > 0.0) {
        Some(p) => format!("超合同界、未到拒收线, 按条款扣 {p:.1} 元/吨"),
        None => "超合同界、未到拒收线, 按条款扣款".into(),
    }
}

/// 捣固 CSR 那一行: 如实说明它不参与达标判定, 合同 CSR 按指标行的值判.
fn stamp_line(
    stamp: f64,
    spec: Option<&Value>,
    csr: Option<&IndicatorCheck>,
    interaction: Option<f64>,
) -> String {
    let mut line = format!(
        "捣固炼焦 CSR 估算 {stamp:.1}: 按捣固公式由配合煤挥发、粘结、胶质另算的参考值, 不参与达标判定"
    );
    let Some(spec) = spec else {
        line.push_str("; 合同没有 CSR 要求");
        return line;
    };
    let basis = match csr.map(|c| c.method) {
        Some(EvaluationMethod::Regression) => "按实测数据回归的模型值".to_string(),
        _ => match interaction.filter(|p| *p > 0.0) {
            Some(p) => format!("各煤 CSR 按配比加权, 再扣煤阶不相容 {p:.1} 点"),
            None => "各煤 CSR 按配比加权".to_string(),
        },
    };
    line.push_str(&format!(
        "; 合同的焦炭强度 CSR 一项按上面指标行的 CSR 值判定 ({basis})"
    ));
    let min = spec["min"]
        .as_f64()
        .filter(|_| spec["direction"] != "Upper");
    if let Some(min) = min.filter(|min| stamp < *min) {
        line.push_str(&format!("。注意: 这个捣固估算值低于合同下限 {min}"));
    }
    line
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

/// 这种煤为什么 (不) 参与今日求解.
fn pool_state(coal: &today::ResolvedCoal) -> String {
    match today::idle_reason(coal) {
        None => "参与今日求解".into(),
        Some(reason) => format!("不参与求解: {reason}"),
    }
}

fn find_coal(args: &Map<String, Value>, context: &Context) -> Result<String, String> {
    let raw = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or("find_coal 需要 name")?;
    let query = today::normalize(raw);
    // 与 today_blend 同一套解析: 指标按用户在煤池改过的值.
    let pool = today::resolve_pool(&context.master, &context.storage)?;
    let matches: Vec<&today::ResolvedCoal> = pool
        .iter()
        .filter(|coal| today::normalize(&coal.name).contains(&query))
        .take(10)
        .collect();
    if matches.is_empty() {
        return Err(format!("煤库里没有名字含「{raw}」的煤"));
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
        let entry = coal.entry;
        let name = coal.name.as_str();
        let status = match entry["status"].as_str() {
            Some("verified") | Some("active") => "在用",
            Some("archived") => "已停用",
            Some("draft") => "待核实",
            Some("incomplete") => "数据不全",
            _ => "未知",
        };
        let spec_fields: Vec<&str> = entry["confidence"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, level)| level.as_str() == Some("spec"))
            .map(|(k, _)| k.as_str())
            .collect();
        let props: Vec<String> = today::INDICATORS
            .iter()
            .filter_map(|k| {
                coal.props.get(*k).and_then(Value::as_f64).map(|v| {
                    let mark = if coal.overridden.iter().any(|o| o == k) {
                        match entry["props"][*k].as_f64() {
                            Some(original) => format!("(用户改过, 原 {original})"),
                            None => "(用户补填)".to_string(),
                        }
                    } else if spec_fields.contains(k) {
                        "(规格)".to_string()
                    } else {
                        String::new()
                    };
                    format!("{} {v}{mark}", label(k))
                })
            })
            .collect();
        let tags: Vec<&str> = ["coal_type", "form", "region"]
            .iter()
            .filter_map(|key| entry[*key].as_str())
            .collect();
        let mut line = format!("{name} [{status}] {}", tags.join(" "));
        line.push_str(&format!("\n  煤池: {}", pool_state(coal)));
        line.push_str(&format!("\n  指标: {}", props.join(", ")));
        if let Some(factor) = &factor {
            // 带上煤种、形态、地区: 估价按它们逐层对标基准, 与 today_blend 同一口径.
            let as_coal: Result<blend_kit::Coal, _> = serde_json::from_value(json!({
                "name": name, "props": coal.props, "fob": 0.0, "frt": 0.0,
                "coal_type": entry["coal_type"], "form": entry["form"], "region": entry["region"]
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
        if let Some(note) = entry["note"].as_str() {
            line.push_str(&format!("\n  备注: {note}"));
        }
        out.push(line);
    }
    if spec_fields_legend_needed(&out) {
        out.push("(规格) = 合同/报价的保证界限, 不是化验值".into());
    }
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
        let mut line = format!(
            "- {} {} ({})",
            label(spec["indicator"].as_str().unwrap_or("")),
            bound_text(spec),
            rule_text(spec, &acceptance(spec))
        );
        if spec["enforcement"] == "Priced" {
            match spec["penalty"]["reject"].as_f64() {
                Some(reject) => line.push_str(&format!(
                    "; 超出合同界按条款扣款, 到 {} 拒收",
                    trim_num(reject, 4)
                )),
                None => line.push_str("; 超出合同界按条款扣款"),
            }
        }
        out.push(line);
    }
    if context.benchmarks.is_empty() {
        out.push(match context.market_price {
            Some(m) => format!(
                "估价市场价: 标准品 1000 点 = {m:.0} (焦煤现货指数点位, 无基准报价时的退路)"
            ),
            None => "估价市场价: 没有基准报价, 现货指数也取不到".into(),
        });
    } else {
        out.push("估价基准报价 (按指标距离对最接近的几条加权换算):".into());
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
    fn fabricated(specs: Value) -> today::TodayRequest {
        today::TodayRequest {
            request: json!({"coals": [], "specs": specs, "total_quantity": 900.0,
                            "price_factor": {"extra_coals": []}}),
            contract_name: "测试合同".into(),
            notes: Vec::new(),
            excluded: Vec::new(),
            excluded_idle: Vec::new(),
        }
    }

    fn check(indicator: &str, value: f64, judged: f64, status: &str, method: &str) -> Value {
        json!({"indicator": indicator, "label_zh": indicator, "value": value, "binding": false,
               "proxy_value": value, "evaluated_value": value, "judged_value": judged,
               "status": status, "method": method})
    }

    #[test]
    fn test_unknown_names_are_refused_with_candidates() {
        let (text, is_error) = call("today_blend", json!({"exclude": ["硫"]}));
        assert!(is_error, "{text}");
        assert!(text.contains("没有叫「硫」的煤"), "{text}");
        assert!(text.contains("名字里带「硫」的有: 低硫、高硫"), "{text}");
        assert!(!text.contains("今日配方"), "对不上名字就不该求解: {text}");

        let (text, is_error) = call("check_ratios", json!({"ratios": {"硫": 1, "低硫": 1}}));
        assert!(is_error, "{text}");
        assert!(text.contains("没有叫「硫」的煤"), "{text}");
        assert!(!text.contains("无解"), "煤名不对不是无解: {text}");

        let (text, _) = call("today_blend", json!({"exclude": ["焦"]}));
        assert!(text.contains("find_coal"), "{text}");
    }

    #[test]
    fn test_exclude_lists_only_real_exclusions() {
        let (text, is_error) = call("today_blend", json!({"exclude": ["高硫", "停产"]}));
        assert!(!is_error, "{text}");
        assert!(text.contains("本次排除: 高硫\n"), "{text}");
        assert!(
            text.contains("要排除的煤本来就不参与求解: 停产 (煤矿已停用)"),
            "{text}"
        );
    }

    #[test]
    fn test_check_ratios_uses_pool_names_and_refuses_idle_coals() {
        let mut context = context();
        let coals = context.master["coals"].as_array_mut().unwrap();
        coals.push(json!({"name": "ABC", "status": "verified", "props": {"S": 0.6}}));
        coals.push(json!({"name": "藏硫", "status": "verified", "props": {"S": 0.6}}));
        context.storage.coal_prefs["藏硫"] = json!({"hidden": true});

        let (text, is_error) = call_with(
            &context,
            "check_ratios",
            json!({"ratios": {"abc ": 1, "高硫": 1}}),
        );
        assert!(!is_error, "{text}");
        assert!(
            text.contains("配比验算") && text.contains("- ABC "),
            "{text}"
        );
        assert!(!text.contains("无解"), "{text}");

        let (text, is_error) = call_with(
            &context,
            "check_ratios",
            json!({"ratios": {"停产": 1, "低硫": 1}}),
        );
        assert!(is_error, "{text}");
        assert!(
            text.contains("「停产」在煤池里但不参与求解: 煤矿已停用"),
            "{text}"
        );
        assert!(!text.contains("无解"), "{text}");

        // 候选只列能参与求解的煤; 排除时照列所有名字相近的煤.
        let (text, _) = call_with(&context, "check_ratios", json!({"ratios": {"硫": 1}}));
        assert!(text.contains("名字里带「硫」的有: 低硫、高硫\n"), "{text}");
        let (text, _) = call_with(&context, "today_blend", json!({"exclude": ["硫"]}));
        assert!(
            text.contains("名字里带「硫」的有: 低硫、高硫、藏硫"),
            "{text}"
        );
    }

    #[test]
    fn test_estimated_rows_outside_raw_bound_say_so() {
        let built = fabricated(json!([
            {"indicator": "V", "direction": "Upper", "max": 28.0,
             "acceptance": {"mode": "Raw", "tolerance": 0.5}},
            {"indicator": "G", "direction": "Lower", "min": 80.0,
             "acceptance": {"mode": "Round", "decimals": 1}},
            {"indicator": "Y", "direction": "Lower", "min": 15.0},
            {"indicator": "CSR", "direction": "Lower", "min": 60.0, "enforcement": "Priced",
             "penalty": {"tiers": [], "reject": 55.0}}
        ]));
        let mut csr = check("CSR", 58.0, 58.0, "Unverified", "ProvisionalLinear");
        csr["penalty_per_ton"] = json!(12.0);
        let result = json!({"ok": true, "recipe": {}, "warnings": [], "indicator_check": [
            check("V", 28.3, 28.3, "Unverified", "ProvisionalLinear"),
            check("G", 79.95, 80.0, "Unverified", "ProvisionalLinear"),
            check("Y", 16.0, 16.0, "Unverified", "ProvisionalLinear"),
            csr,
        ]});
        let text = format_result(&built, &result, None);
        let beyond = "界内 (估算值; 原值超出合同数, 按合同判定规则算界内)";
        assert!(
            text.contains(&format!("- 挥发 V 28.30 (合同 ≤28): {beyond}")),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "- 粘结 G 79.95, 按合同规则记 80.0 (合同 ≥80): {beyond}"
            )),
            "{text}"
        );
        assert!(
            text.contains("- 胶质 Y 16.00 (合同 ≥15): 界内 (估算值)\n"),
            "{text}"
        );
        assert!(
            text.contains("- 焦炭强度 CSR 58.00 (合同 ≥60): 超合同界、未到拒收线, 按条款扣 12.0 元/吨 (估算值)"),
            "{text}"
        );
    }

    #[test]
    fn test_estimated_row_outside_bound_through_solver() {
        let mut context = context();
        context.master["coals"][0]["props"]["V"] = json!(28.3);
        context.storage.contract = Some(json!([
            {"indicator": "V", "direction": "Upper", "max": 28.0,
             "acceptance": {"mode": "Raw", "tolerance": 0.5}}
        ]));
        let (text, is_error) = call_with(&context, "check_ratios", json!({"ratios": {"低硫": 1}}));
        assert!(!is_error, "{text}");
        assert!(
            text.contains(
                "- 挥发 V 28.30 (合同 ≤28): 界内 (估算值; 原值超出合同数, 按合同判定规则算界内)"
            ),
            "{text}"
        );
    }

    #[test]
    fn test_shown_value_never_judges_differently() {
        let built = fabricated(json!([{"indicator": "M", "direction": "Upper", "max": 8.0}]));
        let result = json!({"ok": true, "recipe": {}, "warnings": [], "indicator_check": [
            check("M", 7.9999995, 7.9, "Pass", "Linear")]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains("- 水分 M 7.999999, 按合同规则记 7.9 (合同 ≤8): 达标"),
            "{text}"
        );

        // 求解器残差让原值略过 8, 核心按放行量仍记 7.9: 显示值也要落在 7.9 那一格.
        let result = json!({"ok": true, "recipe": {}, "warnings": [], "indicator_check": [
            check("M", 8.0000002, 7.9, "TolerancePass", "Linear")]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains("- 水分 M 7.999999, 按合同规则记 7.9"),
            "{text}"
        );

        let spec = json!({"indicator": "M", "direction": "Upper", "max": 8.0});
        for mode in ["Truncate", "Round"] {
            let rule: AcceptanceRule =
                serde_json::from_value(json!({"mode": mode, "decimals": 1})).unwrap();
            for (center, step) in [8.0, 8.05]
                .into_iter()
                .flat_map(|c| (-2000..=2000).map(move |s| (c, s)))
            {
                let value = center + f64::from(step) * 1.7e-9;
                for nudge in [-1e-6, 0.0, 1e-6] {
                    let judged = quantize(value + nudge, &rule);
                    let shown = display_value(value, Some(judged), Some(&spec), Some(&rule));
                    let parsed: f64 = shown.parse().unwrap();
                    assert!(
                        (quantize(parsed, &rule) - judged).abs() < 1e-9,
                        "{mode} {value} 判 {judged}, 显示 {shown}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_undetermined_result_is_not_called_infeasible() {
        let built = fabricated(json!([{"indicator": "S", "direction": "Upper", "max": 1.0}]));
        let result = json!({"ok": false, "undetermined": true, "warnings": [],
            "reason": "求解器没有算出可靠结论 (迭代次数用尽), 无法判定合同是否可行",
            "infeasible_bounds": []});
        let text = format_result(&built, &result, None);
        assert!(text.contains("这次没算出结论"), "{text}");
        assert!(text.contains("这不等于合同配不出来"), "{text}");
        assert!(!text.contains("无解"), "{text}");
    }

    #[test]
    fn test_infeasible_bounds_are_plain_words() {
        let (text, _) = call("today_blend", json!({"exclude": ["低硫"]}));
        assert!(
            text.contains("- 硫 S: 合同要求 ≤1.5, 其余要求都满足时这批煤最好只能做到 2.4"),
            "{text}"
        );
        assert!(text.contains("写成 ≤2.4 才配得出"), "{text}");
        assert!(!text.contains('{'), "不得吐原始 JSON: {text}");

        // relax_to 比 achievable 还小: 截断规则下 ≤1.2 放行到 1.2999, 要说清不矛盾.
        let built = fabricated(json!([{"indicator": "S", "direction": "Upper", "max": 1.0}]));
        let result = json!({"ok": false, "reason": "约束冲突", "warnings": [],
            "infeasible_bounds": [{"achievable": 1.2565290557686786, "direction": "Upper",
                "enforced": 1.09999, "indicator": "S", "label_zh": "硫", "margin": 0.0,
                "relax_to": 1.2, "required": 1.0}]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains(
                "- 硫 S: 合同要求 ≤1, 其余要求都满足时这批煤最好只能做到 1.26; \
                 合同上这一项写成 ≤1.2 才配得出 (这是合同上要填的数, 已按本项判定规则\
                 「保留 1 位小数、截断后判定」逐档真解过, 所以比 1.26 还小也配得出)"
            ),
            "{text}"
        );
        assert!(!text.contains("1.09999"), "执行界不展示: {text}");

        let result = json!({"ok": false, "reason": "约束冲突", "warnings": [],
            "infeasible_bounds": [{"achievable": 66.2, "direction": "Lower", "enforced": 68.5,
                "indicator": "CSR", "label_zh": "CSR", "margin": 0.5, "relax_to": null,
                "required": 68.0}]});
        let built = fabricated(json!([{"indicator": "CSR", "direction": "Lower", "min": 68.0}]));
        let text = format_result(&built, &result, None);
        assert!(
            text.contains("合同要求 ≥68, 其余要求都满足时这批煤最好只能做到 66.2"),
            "{text}"
        );
        assert!(text.contains("只放宽这一项还配不出来"), "{text}");
        assert!(text.contains("安全余量 0.5"), "{text}");

        // 按原值判定、合同另有允许偏差: relax_to 比 achievable 小是因为允许偏差.
        let built = fabricated(json!([{"indicator": "M", "direction": "Upper", "max": 10.0,
            "acceptance": {"mode": "Raw", "tolerance": 0.5}}]));
        let result = json!({"ok": false, "reason": "约束冲突", "warnings": [],
            "infeasible_bounds": [{"achievable": 10.8, "direction": "Upper", "enforced": 10.5,
                "indicator": "M", "label_zh": "水分", "margin": 0.0, "relax_to": 10.3,
                "required": 10.0}]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains(
                "- 水分 M: 合同要求 ≤10, 其余要求都满足时这批煤最好只能做到 10.8; \
                 合同上这一项写成 ≤10.3 才配得出 (这是合同上要填的数: 本项按原值判定, \
                 但合同另允许超出上限 0.5 以内, 所以比 10.8 还小也配得出)"
            ),
            "{text}"
        );
        assert!(!text.contains("不取舍"), "{text}");

        // 计价项卡住的是拒收线.
        let built = fabricated(json!([{"indicator": "S", "direction": "Upper", "max": 1.0,
            "enforcement": "Priced", "penalty": {"tiers": [], "reject": 1.3}}]));
        let result = json!({"ok": false, "reason": "约束冲突", "warnings": [],
            "infeasible_bounds": [{"achievable": 1.45, "direction": "Upper", "enforced": 1.39999,
                "indicator": "S", "label_zh": "硫", "margin": 0.0, "relax_to": 1.4,
                "required": 1.3}]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains(
                "- 硫 S: 拒收线 ≤1.3, 其余要求都满足时这批煤最好只能做到 1.45; \
                 拒收线放宽到 ≤1.4 才配得出 (这是拒收线要填的数, 已按本项判定规则\
                 「保留 1 位小数、截断后判定」逐档真解过, 所以比 1.45 还小也配得出)"
            ),
            "{text}"
        );
        assert!(!text.contains("合同上这一项"), "{text}");
    }

    #[test]
    fn test_contract_states_acceptance_rules() {
        let (text, _) = call("get_contract", json!({}));
        assert!(
            text.contains("- 硫 S ≤1.5 (保留 1 位小数、截断后判定: 1.59 记作 1.5)"),
            "{text}"
        );

        let mut context = context();
        context.storage.contract = Some(json!([
            {"indicator": "A", "direction": "Upper", "max": 10.0,
             "acceptance": {"mode": "Round", "decimals": 2}},
            {"indicator": "CSR", "direction": "Lower", "min": 68.0},
            {"indicator": "petro", "direction": "Upper", "max": 0.3},
            {"indicator": "M", "direction": "Upper", "max": 10.0,
             "acceptance": {"mode": "Raw", "tolerance": 0.5}},
            {"indicator": "S", "direction": "Upper", "max": 1.0, "enforcement": "Priced",
             "penalty": {"tiers": [], "reject": 1.3}}
        ]));
        let (text, _) = call_with(&context, "get_contract", json!({}));
        assert!(
            text.contains("- 灰 A ≤10 (保留 2 位小数、四舍五入后判定: 10.004 记作 10.00)"),
            "{text}"
        );
        assert!(
            text.contains(
                "- 焦炭强度 CSR ≥68 (保留 1 位小数、截断后判定: 67.99 记作 67.9, 所以不够 ≥68)"
            ),
            "{text}"
        );
        assert!(
            text.contains("- 岩相 σ ≤0.3 (按原值判定, 不取舍)"),
            "{text}"
        );
        assert!(
            text.contains("- 水分 M ≤10 (按原值判定, 不取舍; 合同另允许超出上限 0.5 以内)"),
            "{text}"
        );
        assert!(text.contains("到 1.3 拒收"), "{text}");
    }

    #[test]
    fn test_indicator_rows_show_true_and_judged_values() {
        let (text, _) = call("today_blend", json!({}));
        assert!(
            text.contains(", 按合同规则记 1.5 (合同 ≤1.5): 达标"),
            "{text}"
        );

        let built = fabricated(json!([
            {"indicator": "A", "direction": "Upper", "max": 10.0},
            {"indicator": "V", "direction": "Upper", "max": 28.0},
            {"indicator": "M", "direction": "Upper", "max": 10.0}
        ]));
        let result = json!({"ok": true, "recipe": {}, "warnings": [], "indicator_check": [
            check("A", 10.0999, 10.0, "TolerancePass", "Linear"),
            check("S", 0.8, 0.8, "Pass", "Linear"),
            check("V", 24.56, 24.5, "Unverified", "ProvisionalLinear"),
            check("G", 80.0, 80.0, "Unverified", "ProvisionalLinear"),
            check("M", 8.0, 8.0, "Pass", "Linear"),
        ]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains("- 灰 A 10.0999, 按合同规则记 10.0 (合同 ≤10): 达标 (原值超出合同数, 按合同判定规则算达标)"),
            "{text}"
        );
        assert!(text.contains("- 硫 S 0.80: 合同无要求"), "{text}");
        assert!(
            text.contains("- 挥发 V 24.56, 按合同规则记 24.5 (合同 ≤28): 界内 (估算值)"),
            "{text}"
        );
        assert!(
            text.contains("- 粘结 G 80.00: 合同无要求 (估算值)"),
            "{text}"
        );
        // 判定值与真实值相同不重复.
        assert!(text.contains("- 水分 M 8.00 (合同 ≤10): 达标"), "{text}");

        let result = json!({"ok": true, "recipe": {}, "warnings": [], "indicator_check": [
            check("A", 10.1001, 10.1, "Fail", "Linear")]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains("- 灰 A 10.10, 按合同规则记 10.1 (合同 ≤10): 不达标"),
            "{text}"
        );
    }

    #[test]
    fn test_display_digits_keep_conclusions_apart() {
        let spec = json!({"indicator": "A", "direction": "Upper", "max": 10.0});
        let rule = acceptance(&spec);
        // 两位小数都显示 10.10, 判定却一个 10.0 一个 10.1: 必须多给位数分开.
        let low = display_value(10.09999, Some(10.0), Some(&spec), Some(&rule));
        let high = display_value(10.10001, Some(10.1), Some(&spec), Some(&rule));
        assert_ne!(low, high);
        let raw = json!({"indicator": "petro", "direction": "Upper", "max": 0.3});
        let raw_rule = acceptance(&raw);
        let shown = display_value(0.3004, Some(0.3004), Some(&raw), Some(&raw_rule));
        assert_eq!(shown, "0.3004");
        // 原值在界外一点点: 四舍五入会显示成界上的数, 要往界外取.
        let shown = display_value(0.30000001, Some(0.30000001), Some(&raw), Some(&raw_rule));
        assert!(!within(shown.parse().unwrap(), &raw, 0.0), "{shown}");
    }

    #[test]
    fn test_unavailable_indicator_is_not_judged() {
        let built = fabricated(json!([{"indicator": "Y", "direction": "Lower", "min": 15.0}]));
        let result = json!({"ok": true, "recipe": {}, "warnings": [], "indicator_check": [
            {"indicator": "Y", "label_zh": "胶质", "value": 0.0, "binding": false,
             "status": "Unverified", "method": "Unavailable"}]});
        let text = format_result(&built, &result, None);
        assert!(
            text.contains("- 胶质 Y (合同 ≥15): 无法判定 (有参配煤缺这项数据)"),
            "{text}"
        );
    }

    #[test]
    fn test_stamp_csr_is_explained() {
        let built = fabricated(json!([{"indicator": "CSR", "direction": "Lower", "min": 68.0}]));
        let result = json!({"ok": true, "recipe": {}, "warnings": [],
            "csr_stamp_estimate": 62.3,
            "indicator_check": [check("CSR", 68.0, 68.0, "Unverified", "ProvisionalLinear")]});
        let text = format_result(&built, &result, None);
        assert!(text.contains("捣固炼焦 CSR 估算 62.3"), "{text}");
        assert!(text.contains("不参与达标判定"), "{text}");
        assert!(
            text.contains("合同的焦炭强度 CSR 一项按上面指标行的 CSR 值判定 (各煤 CSR 按配比加权)"),
            "{text}"
        );
        assert!(text.contains("低于合同下限 68"), "{text}");

        let built = fabricated(json!([]));
        let text = format_result(&built, &result, None);
        assert!(text.contains("合同没有 CSR 要求"), "{text}");
        assert!(!text.contains("低于合同下限"), "{text}");
    }

    #[test]
    fn test_shovel_warning_sits_above_recipe() {
        let mut built = fabricated(json!([]));
        built.notes.push("数据非法未参与: 某煤".into());
        let result = json!({"ok": true, "recipe": {"甲": 0.6, "乙": 0.4},
            "cost": {"cif_per_ton": 1500.0}, "indicator_check": [],
            "warnings": ["剔除 丙: 缺指标 G", "凑不成整铲 (一铲 3 吨): 显示的是未凑整的配方"]});
        let text = format_result(&built, &result, None);
        let warning = text
            .find("注意: 凑不成整铲 (一铲 3 吨): 显示的是未凑整的配方。下面的吨数没有按整铲取整, 不能直接照此装车")
            .unwrap_or_else(|| panic!("{text}"));
        let recipe = text.find("配方:").unwrap();
        assert!(warning < recipe, "{text}");
        let tips = &text[text.find("提示:").unwrap()..];
        assert!(!tips.contains("凑不成整铲"), "{text}");
        assert!(tips.contains("剔除 丙"), "{text}");
    }

    #[test]
    fn test_find_coal_shows_user_overrides() {
        let (before, _) = call("find_coal", json!({"name": "低硫"}));
        let mut context = context();
        context.storage.coal_prefs["低硫"]["props_override"] = json!({"S": 0.9});
        let (text, is_error) = call_with(&context, "find_coal", json!({"name": "低硫"}));
        assert!(!is_error, "{text}");
        assert!(text.contains("硫 S 0.9(用户改过, 原 0.6)"), "{text}");
        assert!(!text.contains("(规格)"), "改过的值不再是规格值: {text}");
        assert!(text.contains("煤池: 参与今日求解"), "{text}");
        let price = |t: &str| t[t.find("估算出厂价").unwrap()..].to_string();
        assert_ne!(price(&before), price(&text), "估价要按改过的值算");

        let (text, _) = call_with(&context, "find_coal", json!({"name": "停产"}));
        assert!(text.contains("不参与求解: 煤矿已停用"), "{text}");
    }
}
