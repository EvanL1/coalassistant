//! 询价精算 (`quote_blend`): 客户问到实际出厂价与运费后, 只用报了价的煤、按到厂价重算配方.
//! 请求由 [`crate::today`] 组装 (对照前端 `domain/blendRequest.ts` 的 `quoteRequest`); 这里解析
//! 参数、排版报价与估算价的对比, 并算出要存档的那一批报价 —— 落库是调用方的事.
use serde_json::{json, Map, Value};

use super::{
    assemble, calls, coal_types, format_result, label, solve, specs_arg, trim_num, typed, Context,
};
use crate::quotes::{self, QuoteBatch, QuoteItem};
use crate::today::{self, Quote, QuotedCoal, TodayOptions, TodayRequest};

/// 占比不超过它的煤不算进了配方 (同配方行的取舍).
const RECIPE_GRAIN: f64 = 1e-4;

/// `tools/list` 里 `quote_blend` 的条目; `quantity` / `specs` 是与其他工具共用的参数说明.
pub(super) fn tool(quantity: &Value, specs: &Value) -> Value {
    json!({
        "name": "quote_blend",
        "description": "询价精算: 客户问到几种煤的实际出厂价与运费后, 只用这几种煤、按到厂价 (出厂价 + 运费) 求最低成本配方。返回配方、成本、8 项指标体检, 以及每种煤的报价比估算价高或低多少。至少两种煤; 煤池里没启用或隐藏的煤报了价也参与, 已停用、重名、数据非法的不行。这批报价会按批存档, 供后台校准估价; 存没存成看结果最后一行, 没存成不影响结果。不改煤池与合同。",
        "inputSchema": { "type": "object", "required": ["quotes"], "properties": {
            "quotes": { "type": "object",
                        "additionalProperties": { "type": "object", "required": ["fob"], "properties": {
                            "fob": { "type": "number", "description": "出厂价 (含税), 元/吨" },
                            "frt": { "type": "number", "description": "运费, 元/吨; 没问到可省, 按 0 计并在结果里提示" },
                            "props": { "type": "object", "additionalProperties": { "type": "number" },
                                       "description": "这批煤当场给的化验值, 键为 S / A / V / G / Y / petro / CSR / M; 只用于本次计算与存档, 不改煤池" }
                        } },
                        "description": "完整煤名 → 这种煤的报价, 如 {\"明鑫\": {\"fob\": 1850, \"frt\": 60}, \"豹子沟\": {\"fob\": 1720}}; 煤名对不上就不求解, 并列出名字相近的煤" },
            "total_quantity": quantity,
            "specs": specs
        } }
    })
}

/// 存没存档不在这里说: 真正落库在调用方, 存完才知道成败, 那句结论由它补到文本结尾.
pub(super) fn quote_blend(args: &Map<String, Value>, context: &Context) -> Result<String, String> {
    let built = build(args, context)?;
    let result = solve(&built, context)?;
    Ok(format_result(&built, &result, context.market_price))
}

/// 一条 `tools/call quote_blend` 消息要存档的那批报价 (同网页询价精算发给 `POST /api/quotes`
/// 的那份); 不是这个工具、不回包的通知、或参数没过校验 (没求解) 为 None. 无解也存档.
/// 这一批过不过得了存档校验留给落库那一步判, 过不了由它如实说没存成.
pub(crate) fn archive(message: &Value, context: &Context) -> Option<QuoteBatch> {
    if !calls(message, "quote_blend") || message.get("id").is_none_or(Value::is_null) {
        return None;
    }
    let args = message["params"]["arguments"].as_object()?;
    Some(archive_batch(&build(args, context).ok()?, context))
}

fn build(args: &Map<String, Value>, context: &Context) -> Result<TodayRequest, String> {
    let options = TodayOptions {
        total_quantity: args.get("total_quantity").and_then(Value::as_f64),
        specs: specs_arg(args)?,
        quotes: Some(parse_quotes(args)?),
        ..Default::default()
    };
    assemble(context, &options)
}

/// 解析 `quotes`: 完整煤名 → `{fob, frt?, props?}`. 有一项不对就整个拒绝并逐条说明 ——
/// 数值范围与存档 (`POST /api/quotes`) 同一套校验, 免得算出来的配方存不了档.
fn parse_quotes(args: &Map<String, Value>) -> Result<Vec<Quote>, String> {
    let given = args
        .get("quotes")
        .and_then(Value::as_object)
        .filter(|quotes| !quotes.is_empty())
        .ok_or("quote_blend 需要 quotes: 完整煤名 → {fob: 出厂价, frt: 运费 (可省), props: 当场给的化验值 (可省)}")?;
    let mut quotes = Vec::new();
    let mut errors = Vec::new();
    for (name, value) in given {
        let parsed = parse_quote(name, value).and_then(|quote| {
            quotes::validate(&draft_batch(std::slice::from_ref(&quote))).map(|()| quote)
        });
        match parsed {
            Ok(quote) => quotes.push(quote),
            Err(reason) => errors.push(reason),
        }
    }
    if errors.is_empty() {
        // 整批再过一遍: 条数上限之类只有整批才看得出.
        errors.extend(quotes::validate(&draft_batch(&quotes)).err());
    }
    if errors.is_empty() {
        return Ok(quotes);
    }
    Err(format!(
        "报价写得不对, 没有求解:\n- {}\n每种煤给 fob (出厂价, 元/吨, 须为正), 可带 frt (运费, 元/吨) \
         与 props (当场给的化验值, 键为 S / A / V / G / Y / petro / CSR / M).",
        errors.join("\n- ")
    ))
}

fn parse_quote(name: &str, value: &Value) -> Result<Quote, String> {
    let Some(object) = value.as_object() else {
        return Err(format!(
            "「{name}」的报价须为对象, 如 {{\"fob\": 1850, \"frt\": 60}}"
        ));
    };
    let fob = object
        .get("fob")
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("「{name}」缺出厂价 fob (元/吨, 数字)"))?;
    let frt = match object.get("frt") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .ok_or_else(|| format!("「{name}」的运费 frt 须为数字; 没问到就不写, 按 0 计"))?,
        ),
    };
    let props = match object.get("props") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(props)) => props.clone(),
        Some(_) => return Err(format!("「{name}」的 props 须为对象: 指标 → 化验值")),
    };
    Ok(Quote {
        name: name.to_string(),
        fob,
        frt,
        props,
    })
}

/// 只为借存档的校验查数值范围: 煤名、指标照调用方写的, 还没对上煤池.
fn draft_batch(quotes: &[Quote]) -> QuoteBatch {
    let items = quotes
        .iter()
        .map(|quote| {
            item(
                quote.name.clone(),
                quote.fob,
                quote.frt.unwrap_or(0.0),
                &Value::Null,
                quote.props.clone(),
                Vec::new(),
            )
        })
        .collect();
    QuoteBatch {
        batch_id: "mcp".into(),
        items,
    }
}

fn item(
    name: String,
    fob: f64,
    frt: f64,
    tags: &Value,
    props: Map<String, Value>,
    changed: Vec<String>,
) -> QuoteItem {
    let tag = |key: &str| tags[key].as_str().map(String::from);
    QuoteItem {
        name,
        fob,
        frt,
        coal_type: tag("coal_type"),
        form: tag("form"),
        region: tag("region"),
        province: tag("province"),
        city: tag("city"),
        county: tag("county"),
        props,
        changed,
    }
}

/// 要存档的一批: 煤名用煤池里的写法, 指标是这次实际用的 (煤池值 ∪ 当场给的), 标签照煤池.
/// 批号由日期与这组煤定: 同一天同一组煤再算一次覆盖上一次, 换了煤或隔了天另起一批.
fn archive_batch(built: &TodayRequest, context: &Context) -> QuoteBatch {
    let coals = built.request["coals"].as_array();
    let items: Vec<QuoteItem> = built
        .quoted
        .iter()
        .zip(coals.into_iter().flatten())
        .map(|(quote, coal)| {
            let props = coal["props"].as_object().cloned().unwrap_or_default();
            item(
                quote.name.clone(),
                quote.fob,
                quote.frt,
                coal,
                props,
                quote.changed.clone(),
            )
        })
        .collect();
    let mut names: Vec<String> = items
        .iter()
        .map(|item| today::normalize(&item.name))
        .collect();
    names.sort();
    let digest = crate::api_key::hash_key(&names.join("\n"));
    QuoteBatch {
        batch_id: format!("mcp-{}-{}", context.today.format("%Y%m%d"), &digest[..16]),
        items,
    }
}

/// 第一行的计价口径; 不是询价精算为 None.
pub(super) fn pricing_text(built: &TodayRequest, market_text: &str) -> Option<String> {
    if built.quoted.is_empty() {
        return None;
    }
    let benchmarks = built.request["price_factor"]["benchmarks"].as_array();
    let compared = if benchmarks.is_some_and(|list| !list.is_empty()) {
        format!("估算价只作对照 ({market_text})")
    } else {
        "没有基准报价, 这次不比估算价".to_string()
    };
    Some(format!("按你给的报价, 到厂价 = 出厂价 + 运费, {compared}"))
}

/// 成本行后面的口径说明; 不是询价精算为空. 两个工具的成本口径不同 (这里到厂、含运费, 今日配方
/// 按估算出厂价), 不写明的话模型会把两个数相减, 说成 "实价比估价贵多少".
pub(super) fn cost_note(built: &TodayRequest) -> &'static str {
    if built.quoted.is_empty() {
        return "";
    }
    " (到厂口径, 含运费; today_blend 的成本按估算出厂价、不含运费, 两个数不能直接相减)"
}

/// 一种报了价的煤的价格说明: 报价、运费、到厂价, 报价比估算价高或低多少, 当场给的化验值;
/// 不是报了价的煤为 None.
pub(super) fn price_text(built: &TodayRequest, result: &Value, name: &str) -> Option<String> {
    let at = built.quoted.iter().position(|quote| quote.name == name)?;
    let quote = &built.quoted[at];
    let mut text = format!("出厂报价 {} 元/吨", trim_num(quote.fob, 2));
    if quote.frt_given {
        text.push_str(&format!(
            " + 运费 {} = 到厂价 {}",
            trim_num(quote.frt, 2),
            trim_num(quote.fob + quote.frt, 2)
        ));
    } else {
        text.push_str(", 运费没给按 0 计");
    }
    if let Some(gap) = gap_text(result, name) {
        text.push_str(&format!(", {gap}"));
    }
    if let Some(changed) = changed_text(quote, &built.request["coals"][at]) {
        text.push_str(&format!(", {changed}"));
    }
    Some(text)
}

/// "报价比估算出厂价 1795 高 55 元 (对标X)". 只在估算价对上了基准时写: 没对上时核心拿这批
/// 报价自己反推估价, 拿它评价报价是循环论证 (同网页的报价对比).
fn gap_text(result: &Value, name: &str) -> Option<String> {
    let value = result["coal_values"]
        .as_array()?
        .iter()
        .find(|value| value["name"] == name)?;
    let benchmark = value["benchmark"].as_str()?;
    let fair = value["fair"]
        .as_f64()
        .filter(|fair| fair.is_finite() && *fair > 0.0)?;
    // 先取整再判高低: 差 0.3 元写 "持平", 不能是 "高 0 元".
    let gap = value["gap"].as_f64().filter(|gap| gap.is_finite())?.round();
    Some(if gap == 0.0 {
        format!("报价与估算出厂价 {fair:.0} 持平 (对标{benchmark})")
    } else {
        let side = if gap > 0.0 { "高" } else { "低" };
        format!(
            "报价比估算出厂价 {fair:.0} {side} {:.0} 元 (对标{benchmark})",
            gap.abs()
        )
    })
}

/// "化验值按当场给的: 硫 S 0.7、灰 A 9.8"; 没有与煤池不同的为 None.
fn changed_text(quote: &QuotedCoal, coal: &Value) -> Option<String> {
    let changed: Vec<String> = quote
        .changed
        .iter()
        .filter_map(|key| {
            let value = coal["props"][key.as_str()].as_f64()?;
            Some(format!("{} {value}", label(key)))
        })
        .collect();
    (!changed.is_empty()).then(|| format!("化验值按当场给的: {}", changed.join("、")))
}

/// 报了价但没进配方的煤各一行; 无解时没有配方, 改列这次用的全部报价. 不是询价精算为空.
pub(super) fn unused_lines(built: &TodayRequest, result: &Value) -> Vec<String> {
    let solved = result["ok"] == true;
    let used = |name: &str| {
        solved
            && result["recipe"][name]
                .as_f64()
                .is_some_and(|ratio| ratio > RECIPE_GRAIN)
    };
    let types = coal_types(&built.request);
    let rest: Vec<String> = built
        .quoted
        .iter()
        .filter(|quote| !used(&quote.name))
        .filter_map(|quote| {
            let price = price_text(built, result, &quote.name)?;
            Some(format!("- {}: {price}", typed(&quote.name, &types)))
        })
        .collect();
    if rest.is_empty() {
        return rest;
    }
    let head = if solved {
        "报了价但没进配方:"
    } else {
        "这次用的报价:"
    };
    std::iter::once(head.to_string()).chain(rest).collect()
}

#[cfg(test)]
#[path = "mcp_quote_tests.rs"]
mod tests;
