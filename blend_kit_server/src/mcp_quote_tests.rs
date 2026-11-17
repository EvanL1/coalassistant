//! 询价精算的单测: 只用固定数据, 不碰库.
use super::*;
use crate::database::UserStorage;
use serde_json::json;

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
                {"name": "低硫", "status": "active", "coal_type": "焦煤", "province": "山西省",
                 "city": "临汾市", "props": {"S": 0.6}},
                {"name": "高硫", "status": "active", "coal_type": "瘦煤", "props": {"S": 2.4}},
                {"name": "中硫", "status": "active", "coal_type": "焦煤", "props": {"S": 1.2}},
                {"name": "停产", "status": "archived", "props": {"S": 0.1}}
            ]
        }),
        storage: UserStorage {
            initialized: true,
            // 中硫在煤池里没启用.
            coal_prefs: json!({"低硫": {"enabled": true}, "高硫": {"enabled": true}}),
            contract: None,
            quantity: 1000.0,
            user_coals: json!([]),
            shovel_tons: None,
        },
        market_price: None,
        benchmarks: vec![
            json!({"name": "晋中中硫主焦", "coal_type": "焦煤", "props": {"S": 1.3},
                   "price": 2050.0, "date": "2026-10-10"}),
            json!({"name": "吕梁高硫瘦煤", "coal_type": "瘦煤", "props": {"S": 2.8},
                   "price": 1860.0, "date": "2026-10-09"}),
        ],
        rank_k: None,
        coal_prices: None,
        market: None,
        today: "2026-11-14".parse().unwrap(),
    }
}

fn message(arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
           "params": {"name": "quote_blend", "arguments": arguments}})
}

fn call_with(context: &Context, arguments: Value) -> (String, bool) {
    let response = super::super::handle(&message(arguments), Some(context)).unwrap();
    let result = &response["result"];
    (
        result["content"][0]["text"].as_str().unwrap().to_string(),
        result["isError"] == true,
    )
}

fn call(arguments: Value) -> (String, bool) {
    call_with(&context(), arguments)
}

fn two_quotes() -> Value {
    json!({"quotes": {"低硫": {"fob": 2200, "frt": 60}, "高硫": {"fob": 1700, "frt": 40}}})
}

#[test]
fn test_quote_request_puts_quoted_coals_into_the_lp() {
    let context = context();
    let args = json!({
        "quotes": {
            "低硫": {"fob": 2200, "frt": 60, "props": {"S": 0.7, "A": 9.5}},
            "中硫": {"fob": 2100}
        },
        "total_quantity": 500
    });
    let built = build(args.as_object().unwrap(), &context).unwrap();
    let request = &built.request;
    // 报了价的煤带实价进 LP (没启用的中硫也算), 指标 = 煤池值 ∪ 当场给的, 标签照煤池.
    assert_eq!(
        request["coals"],
        json!([
            {"name": "中硫", "props": {"S": 1.2}, "fob": 2100.0, "frt": 0.0, "coal_type": "焦煤"},
            {"name": "低硫", "props": {"S": 0.7, "A": 9.5}, "fob": 2200.0, "frt": 60.0,
             "coal_type": "焦煤", "province": "山西省", "city": "临汾市"}
        ])
    );
    // 不按估算价并入: 煤池里启用的高硫没报价, 不参与.
    assert_eq!(request["price_factor"]["extra_coals"], json!([]));
    assert_eq!(request["price_factor"]["estimate_unpriced"], false);
    assert_eq!(request["total_quantity"], 500.0);

    // 其余与今日求解的请求完全一样.
    let plain = assemble(&context, &TodayOptions::default())
        .unwrap()
        .request;
    for key in ["specs", "shovel_tons", "truncate_decimal"] {
        assert_eq!(request[key], plain[key], "{key}");
    }
    let mut factor = request["price_factor"].clone();
    let mut plain_factor = plain["price_factor"].clone();
    for key in ["extra_coals", "estimate_unpriced"] {
        factor.as_object_mut().unwrap().remove(key);
        plain_factor.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(factor, plain_factor);
    assert!(request.get("fixed_ratios").is_none());
}

#[test]
fn test_quote_blend_reports_quotes_against_estimates() {
    let (text, is_error) = call(two_quotes());
    assert!(!is_error, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "询价精算 (默认合同, 1000 吨, 按你给的报价, 到厂价 = 出厂价 + 运费, \
         估算价只作对照 (按 2 条分煤种基准报价估价, 最新 2026-10-10))"
    );
    assert!(!text.contains("全部按估算价"), "{text}");
    // 估算出厂价: 低硫 2050 × 1052.5 / 1000 = 2158, 高硫 1860 × 862.5 / 812.5 = 1974.
    assert!(
        text.contains(
            "出厂报价 2200 元/吨 + 运费 60 = 到厂价 2260, \
             报价比估算出厂价 2158 高 42 元 (对标晋中中硫主焦))"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "出厂报价 1700 元/吨 + 运费 40 = 到厂价 1740, \
             报价比估算出厂价 1974 低 274 元 (对标吕梁高硫瘦煤))"
        ),
        "{text}"
    );
    // 成本按到厂价: 一铲 3 吨共 333 铲, 低硫 149 铲、高硫 184 铲 (硫卡在 1.6 以下).
    // 成本行注明口径, 免得模型拿它与 today_blend 按估算出厂价 (不含运费) 的成本相减.
    assert!(
        text.contains(
            "成本 1972.7 元/吨, 合计 197.3 万元 (到厂口径, 含运费; today_blend 的成本按估算出厂价、\
             不含运费, 两个数不能直接相减)"
        ),
        "{text}"
    );
    assert!(!text.contains("没给运费"), "{text}");
    // 存没存成由落库那一步在回包结尾补写, 这里不写存档结论.
    assert!(!text.contains("存档"), "{text}");
}

#[test]
fn test_cost_note_only_for_quote_blend() {
    assert_eq!(cost_note(&TodayRequest::default()), "");
    let quoted = |name: &str| QuotedCoal {
        name: name.into(),
        fob: 1.0,
        frt: 0.0,
        frt_given: false,
        changed: Vec::new(),
    };
    let built = TodayRequest {
        quoted: vec![quoted("甲")],
        ..Default::default()
    };
    assert!(cost_note(&built).contains("到厂口径"));
}

#[test]
fn test_gap_text_rounds_first_and_needs_a_benchmark() {
    let result = |gap: f64, benchmark: Value| {
        let mut result = BlendResult::infeasible("", Vec::new());
        result.coal_values = serde_json::from_value(json!([
            {"name": "甲", "fob": 2000.0 + gap, "fair": 2000.0, "gap": gap, "benchmark": benchmark,
             "adjustment": 0.0, "points": 1000.0}
        ]))
        .unwrap();
        result
    };
    assert_eq!(
        gap_text(&result(54.6, json!("基准")), "甲").unwrap(),
        "报价比估算出厂价 2000 高 55 元 (对标基准)"
    );
    assert_eq!(
        gap_text(&result(-8.2, json!("基准")), "甲").unwrap(),
        "报价比估算出厂价 2000 低 8 元 (对标基准)"
    );
    assert_eq!(
        gap_text(&result(-0.3, json!("基准")), "甲").unwrap(),
        "报价与估算出厂价 2000 持平 (对标基准)"
    );
    // 没对上基准: 估价是拿这批报价反推的, 不比.
    assert_eq!(gap_text(&result(54.6, Value::Null), "甲"), None);
    assert_eq!(gap_text(&result(54.6, json!("基准")), "乙"), None);
}

#[test]
fn test_quote_without_benchmarks_does_not_compare() {
    let mut context = context();
    context.benchmarks.clear();
    context.market_price = Some(2000.0);
    let (text, is_error) = call_with(&context, two_quotes());
    assert!(!is_error, "{text}");
    assert!(
        text.starts_with(
            "询价精算 (默认合同, 1000 吨, 按你给的报价, 到厂价 = 出厂价 + 运费, \
             没有基准报价, 这次不比估算价)\n"
        ),
        "{text}"
    );
    // 不报 "比估算出厂价高/低" (成本行那句口径说明不在此列).
    assert!(!text.contains("比估算出厂价"), "{text}");
    assert!(
        text.contains("出厂报价 2200 元/吨 + 运费 60 = 到厂价 2260)"),
        "{text}"
    );
}

#[test]
fn test_missing_freight_counts_as_zero_and_is_flagged() {
    let (text, is_error) =
        call(json!({"quotes": {"低硫": {"fob": 2200, "frt": 60}, "高硫": {"fob": 1700}}}));
    assert!(!is_error, "{text}");
    assert!(
        text.contains("出厂报价 1700 元/吨, 运费没给按 0 计"),
        "{text}"
    );
    assert!(
        text.contains("\n提示:\n- 1 种煤没给运费, 按 0 计: 高硫\n"),
        "{text}"
    );
    // 运费写 0 是给了运费, 不提示.
    let (text, _) =
        call(json!({"quotes": {"低硫": {"fob": 2200, "frt": 0}, "高硫": {"fob": 1700, "frt": 0}}}));
    assert!(!text.contains("没给运费"), "{text}");
    assert!(
        text.contains("出厂报价 1700 元/吨 + 运费 0 = 到厂价 1700"),
        "{text}"
    );
}

#[test]
fn test_quoted_coals_left_out_of_the_recipe_are_listed() {
    let (text, is_error) = call(json!({
        "quotes": {
            "低硫": {"fob": 2200, "frt": 60, "props": {"S": 0.7}},
            "高硫": {"fob": 1700, "frt": 40},
            "中硫": {"fob": 2500, "frt": 40}
        },
        "specs": {"S": {"max": 1.8}}
    }));
    assert!(!is_error, "{text}");
    assert!(
        text.starts_with("询价精算 (默认合同; 本次改: 硫 S ≤1.8, 1000 吨, 按你给的报价"),
        "{text}"
    );
    // 中硫没启用也参与, 但太贵没进配方: 2050 × 1007.5 / 1000 = 2065.
    assert!(
        text.contains(
            "\n报了价但没进配方:\n- 中硫 (焦煤): 出厂报价 2500 元/吨 + 运费 40 = 到厂价 2540, \
             报价比估算出厂价 2065 高 435 元 (对标晋中中硫主焦)\n"
        ),
        "{text}"
    );
    // 当场给的化验值进了本次计算: 低硫的估算价按 S 0.7 算, 2050 × 1045 / 1000 = 2142.
    assert!(
        text.contains(
            "报价比估算出厂价 2142 高 58 元 (对标晋中中硫主焦), 化验值按当场给的: 硫 S 0.7)"
        ),
        "{text}"
    );
}

#[test]
fn test_infeasible_quote_lists_the_quotes_and_still_archives() {
    let args = json!({
        "quotes": {"低硫": {"fob": 2200, "frt": 60}, "高硫": {"fob": 1700, "frt": 40}},
        "specs": {"S": {"max": 0.3}}
    });
    let (text, is_error) = call(args.clone());
    assert!(!is_error, "{text}");
    assert!(text.contains("无解"), "{text}");
    assert!(
        text.contains(
            "\n这次用的报价:\n- 低硫 (焦煤): 出厂报价 2200 元/吨 + 运费 60 = 到厂价 2260"
        ),
        "{text}"
    );
    assert!(!text.contains("没进配方"), "{text}");
    assert!(!text.contains("存档"), "{text}");
    assert_eq!(archive(&message(args), &context()).unwrap().items.len(), 2);
}

#[test]
fn test_quote_needs_at_least_two_coals() {
    let (text, is_error) = call(json!({"quotes": {"低硫": {"fob": 2200}}}));
    assert!(is_error, "{text}");
    assert_eq!(text, "询价精算至少要两种煤的报价, 这次只有 1 种");

    for arguments in [json!({}), json!({"quotes": {}}), json!({"quotes": [1, 2]})] {
        let (text, is_error) = call(arguments.clone());
        assert!(
            is_error && text.contains("quote_blend 需要 quotes"),
            "{text}"
        );
        assert!(archive(&message(arguments), &context()).is_none());
    }
    // 同一种煤写了两遍 (只差空格) 不算两种.
    let (text, is_error) = call(json!({"quotes": {"低硫": {"fob": 2200}, " 低硫": {"fob": 2210}}}));
    assert!(is_error && text.contains("煤名重复: 低硫"), "{text}");
}

#[test]
fn test_bad_quotes_are_rejected_without_solving_or_archiving() {
    let cases = [
        (json!({"fob": 0}), "低硫 的出厂价须在 0–100000 元之间"),
        (json!({"fob": -5}), "低硫 的出厂价须在 0–100000 元之间"),
        (json!({"fob": 100000}), "低硫 的出厂价须在 0–100000 元之间"),
        (json!({"fob": "2200"}), "「低硫」缺出厂价 fob"),
        (json!({"frt": 60}), "「低硫」缺出厂价 fob"),
        (json!(2200), "「低硫」的报价须为对象"),
        (
            json!({"fob": 2200, "frt": -1}),
            "低硫 的运费须在 0–10000 元之间",
        ),
        (
            json!({"fob": 2200, "frt": 10000}),
            "低硫 的运费须在 0–10000 元之间",
        ),
        (
            json!({"fob": 2200, "frt": "x"}),
            "「低硫」的运费 frt 须为数字",
        ),
        (
            json!({"fob": 2200, "props": {"Ro": 1.2}}),
            "低硫 的指标 \"Ro\" 不认识",
        ),
        (
            json!({"fob": 2200, "props": {"S": -1}}),
            "低硫 的指标 S = -1 超出物理量程",
        ),
        (
            json!({"fob": 2200, "props": {"S": "x"}}),
            "低硫 的指标 S 须为数字或留空",
        ),
        (
            json!({"fob": 2200, "props": [1]}),
            "「低硫」的 props 须为对象",
        ),
    ];
    for (quote, expected) in cases {
        let arguments = json!({"quotes": {"低硫": quote, "高硫": {"fob": 1700}}});
        let (text, is_error) = call(arguments.clone());
        assert!(is_error, "{quote}: {text}");
        assert!(
            text.starts_with("报价写得不对, 没有求解:\n") && text.contains(expected),
            "{quote}: {text}"
        );
        assert!(!text.contains("配方"), "{quote}: {text}");
        assert!(
            archive(&message(arguments), &context()).is_none(),
            "{quote}"
        );
    }
    // 几种煤都写错时逐条列出.
    let (text, _) = call(json!({"quotes": {"低硫": {"fob": 0}, "高硫": {"fob": 1700, "frt": -3}}}));
    assert!(
        text.contains("低硫 的出厂价") && text.contains("高硫 的运费"),
        "{text}"
    );
}

#[test]
fn test_quoted_names_follow_the_only_rules() {
    // 没启用的煤报了价就参与.
    let (text, is_error) = call(json!({"quotes": {"低硫": {"fob": 2200}, "中硫": {"fob": 2100}}}));
    assert!(!is_error && text.contains("- 中硫 (焦煤)"), "{text}");
    // 已停用的不行; 对不上的列出相近的煤名.
    let (text, is_error) = call(json!({
        "quotes": {"低硫": {"fob": 2200}, "停产": {"fob": 1700}, "硫": {"fob": 1}}
    }));
    assert!(is_error, "{text}");
    assert!(text.contains("「停产」这次用不了: 煤矿已停用"), "{text}");
    assert!(
        text.contains("没有叫「硫」的煤。名字里带「硫」的有: 低硫、高硫、中硫"),
        "{text}"
    );
    assert!(text.contains("本次没有求解"), "{text}");
}

#[test]
fn test_archive_batch_id_is_stable_per_day_and_coal_set() {
    let context = context();
    let batch_of = |context: &Context, arguments: Value| {
        archive(&message(arguments), context).expect("合法的询价应有存档批")
    };
    let first = batch_of(&context, two_quotes());
    // mcp-<YYYYMMDD>-<16 位十六进制>, 过得了存档校验.
    let (prefix, digest) = first.batch_id.split_at(13);
    assert_eq!(prefix, "mcp-20261114-");
    assert_eq!(digest.len(), 16);
    assert!(digest
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    assert!(quotes::validate(&first).is_ok());

    // 同一天同一组煤: 价格、指标、合同、煤名大小写与空格怎么变都是同一批.
    let again = batch_of(
        &context,
        json!({
            "quotes": {" 高硫": {"fob": 1650}, "低硫 ": {"fob": 2300, "props": {"S": 0.8}}},
            "specs": {"S": {"max": 1.8}}, "total_quantity": 300
        }),
    );
    assert_eq!(again.batch_id, first.batch_id);
    // 换了一组煤、或隔了一天, 另起一批.
    let other = batch_of(
        &context,
        json!({"quotes": {"低硫": {"fob": 2200}, "高硫": {"fob": 1700}, "中硫": {"fob": 2100}}}),
    );
    assert_ne!(other.batch_id, first.batch_id);
    let mut tomorrow = context;
    tomorrow.today = "2026-11-15".parse().unwrap();
    let next = batch_of(&tomorrow, two_quotes());
    assert!(
        next.batch_id.starts_with("mcp-20261115-"),
        "{}",
        next.batch_id
    );
    assert_eq!(next.batch_id[13..], first.batch_id[13..]);
}

#[test]
fn test_archive_items_carry_pool_names_tags_and_real_changes() {
    let arguments = json!({"quotes": {
        // S 与煤池值相同, 不算改过; A 是煤池里没有的, 算.
        " 低硫": {"fob": 2200, "frt": 60, "props": {"S": 0.6, "A": 9.5, "V": null}},
        "高硫": {"fob": 1700}
    }});
    let batch = archive(&message(arguments), &context()).unwrap();
    let low = batch.items.iter().find(|item| item.name == "低硫").unwrap();
    assert_eq!((low.fob, low.frt), (2200.0, 60.0));
    assert_eq!(
        Value::Object(low.props.clone()),
        json!({"S": 0.6, "A": 9.5})
    );
    assert_eq!(low.changed, ["A"]);
    assert_eq!(low.coal_type.as_deref(), Some("焦煤"));
    assert_eq!(low.province.as_deref(), Some("山西省"));
    assert_eq!(low.city.as_deref(), Some("临汾市"));
    assert_eq!((&low.form, &low.region, &low.county), (&None, &None, &None));

    let high = batch.items.iter().find(|item| item.name == "高硫").unwrap();
    assert_eq!((high.fob, high.frt), (1700.0, 0.0));
    assert_eq!(Value::Object(high.props.clone()), json!({"S": 2.4}));
    assert!(high.changed.is_empty());
    assert_eq!(batch.items.len(), 2);
}

#[test]
fn test_archive_only_for_answered_quote_blend_calls() {
    let context = context();
    let mut other = message(two_quotes());
    other["params"]["name"] = json!("today_blend");
    assert!(archive(&other, &context).is_none());
    // 通知不回包, 也就没有精算结果, 不存档.
    let mut notification = message(two_quotes());
    notification.as_object_mut().unwrap().remove("id");
    assert!(archive(&notification, &context).is_none());
    // 没求解的 (煤名对不上、合同写错、只有一种煤) 不存档.
    for arguments in [
        json!({"quotes": {"低硫": {"fob": 2200}, "没有这煤": {"fob": 1700}}}),
        json!({"quotes": {"低硫": {"fob": 2200}, "停产": {"fob": 1700}}}),
        json!({"quotes": {"低硫": {"fob": 2200}}}),
        json!({"quotes": {"低硫": {"fob": 2200}, "高硫": {"fob": 1700}}, "specs": {"X": {"max": 1}}}),
    ] {
        assert!(
            archive(&message(arguments.clone()), &context).is_none(),
            "{arguments}"
        );
        assert!(call(arguments).1);
    }
    assert!(archive(&message(two_quotes()), &context).is_some());
}
