//! 按次覆盖 (`specs` / `only`) 的单测: 只用固定数据.
use super::super::{build, Market, TodayOptions, TodayRequest};
use crate::database::UserStorage;
use serde_json::{json, Map, Value};

fn master() -> Value {
    json!({
        "default_contract": {"name": "默认合同", "specs": [
            {"indicator": "S", "direction": "Upper", "max": 2.0}
        ]},
        "price_factor": {"description": "出处", "adjusts": []},
        "coals": [
            {"name": "甲", "status": "verified", "coal_type": "焦煤", "props": {"S": 1.0}},
            {"name": "乙", "status": "verified", "props": {"S": 0.5}},
            {"name": "丙", "status": "active", "props": {"S": 0.7}},
            {"name": "丁", "status": "verified", "props": {"S": 0.8}},
            {"name": "停了", "status": "archived", "props": {"S": 0.5}},
            {"name": "坏数", "status": "active", "props": {"S": 0.5}},
            {"name": "重名", "status": "verified", "props": {"S": 0.5}},
            {"name": "重名 ", "status": "verified", "props": {"S": 0.5}}
        ]
    })
}

/// 丙没启用, 丁隐藏, 坏数没启用且数据非法.
fn storage() -> UserStorage {
    UserStorage {
        initialized: true,
        coal_prefs: json!({"丁": {"hidden": true}, "坏数": {"props_override": {"S": -1}}}),
        contract: None,
        quantity: 3700.0,
        user_coals: json!([]),
        shovel_tons: None,
    }
}

fn saved_contract() -> Value {
    json!([
        {"indicator": "S", "direction": "Upper", "max": 1.6, "enforcement": "Priced",
         "penalty": {"tiers": [{"over": 0.1, "per_ton": 5.0}], "reject": 2.0}},
        {"indicator": "A", "direction": "Upper", "max": 10.0,
         "acceptance": {"mode": "Round", "decimals": 2}, "margin": 0.1},
        {"indicator": "V", "direction": "Range", "min": 20.0, "max": 30.0},
        {"indicator": "G", "direction": "Lower", "min": 70.0, "enforcement": "Soft"},
        {"indicator": "M", "direction": "Upper", "max": 10.0, "enabled": false}
    ])
}

fn market() -> Market {
    Market {
        benchmarks: Vec::new(),
        market_price: Some(1907.0),
    }
}

fn run(storage: &UserStorage, options: TodayOptions) -> Result<TodayRequest, String> {
    build(&master(), storage, &market(), &options)
}

fn with_specs(specs: Value) -> TodayOptions {
    TodayOptions {
        specs: Some(specs.as_object().unwrap().clone()),
        ..Default::default()
    }
}

fn with_only(names: &[&str]) -> TodayOptions {
    TodayOptions {
        only: Some(names.iter().map(|name| name.to_string()).collect()),
        ..Default::default()
    }
}

fn ratios(pairs: &[(&str, f64)]) -> Option<Map<String, Value>> {
    Some(
        pairs
            .iter()
            .map(|(name, share)| (name.to_string(), json!(share)))
            .collect(),
    )
}

fn pool_names(built: &TodayRequest) -> Vec<&str> {
    built.request["price_factor"]["extra_coals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|coal| coal["name"].as_str().unwrap())
        .collect()
}

#[test]
fn test_request_without_overrides_is_exactly_the_saved_state() {
    let mut storage = storage();
    storage.contract = Some(saved_contract());
    let expected = json!({
        "coals": [],
        "specs": saved_contract(),
        "total_quantity": 3700.0,
        "shovel_tons": 3.0,
        "truncate_decimal": true,
        "price_factor": {
            "adjusts": [],
            "market_price": 1907.0,
            "extra_coals": [
                {"name": "甲", "props": {"S": 1.0}, "fob": 0.0, "frt": 0.0, "coal_type": "焦煤"},
                {"name": "乙", "props": {"S": 0.5}, "fob": 0.0, "frt": 0.0}
            ],
            "estimate_unpriced": true
        }
    });
    // 不传、以及传了空的 specs, 请求都与保存的状态逐字段相同, 也没有任何按次的痕迹.
    for options in [TodayOptions::default(), with_specs(json!({}))] {
        let built = run(&storage, options).unwrap();
        assert_eq!(built.request, expected);
        assert_eq!(built.contract_name, "用户自定义合同");
        assert!(built.spec_changes.is_empty() && built.only.is_empty() && built.quoted.is_empty());
        assert_eq!(built.notes, ["煤池里有重名的煤, 整组未参与: 重名、重名 "]);
    }
}

#[test]
fn test_specs_set_direction_from_the_bounds_given() {
    let mut storage = storage();
    storage.contract = Some(saved_contract());
    let built = run(
        &storage,
        with_specs(json!({
            "V": {"max": 28},                 // 原为区间, 只给上限 → Upper, 下限去掉
            "A": {"min": 8},                  // 原为上限, 只给下限 → Lower, 上限去掉
            "M": {"min": 6, "max": 9},        // 原来停用, 两个都给 → Range 并启用
            "G": null                         // 本次不约束
        })),
    )
    .unwrap();
    let specs = &built.request["specs"];
    assert_eq!(
        specs[2],
        json!({"indicator": "V", "direction": "Upper", "max": 28.0, "enabled": true})
    );
    // 判定规则与安全余量照旧.
    assert_eq!(
        specs[1],
        json!({"indicator": "A", "direction": "Lower", "min": 8.0, "enabled": true,
               "acceptance": {"mode": "Round", "decimals": 2}, "margin": 0.1})
    );
    assert_eq!(
        specs[4],
        json!({"indicator": "M", "direction": "Range", "min": 6.0, "max": 9.0, "enabled": true})
    );
    assert_eq!(specs[3]["enabled"], false);
    assert_eq!(specs[3]["min"], 70.0, "不约束只是停用, 原来的界限不动");
    // 没提到的 S 原样不动 (含计价条款).
    assert_eq!(specs[0], saved_contract()[0]);
    assert_eq!(specs.as_array().unwrap().len(), 5);
    // 改过的指标按指标的固定顺序.
    assert_eq!(built.spec_changes, ["A", "V", "G", "M"]);
    assert!(built.notes.iter().all(|note| !note.contains("硬约束")));
    assert_eq!(built.contract_name, "用户自定义合同");
}

#[test]
fn test_spec_on_a_priced_item_becomes_hard_and_says_so() {
    let mut storage = storage();
    storage.contract = Some(saved_contract());
    let built = run(&storage, with_specs(json!({"S": {"max": 1.2}}))).unwrap();
    assert_eq!(
        built.request["specs"][0],
        json!({"indicator": "S", "direction": "Upper", "max": 1.2, "enabled": true})
    );
    assert_eq!(built.notes[0], "硫 S 本次按硬约束算, 未计扣款条款");
    assert_eq!(built.spec_changes, ["S"]);

    // 关掉计价项不用说明: 这一项这次根本不约束.
    let built = run(&storage, with_specs(json!({"S": null}))).unwrap();
    assert_eq!(built.request["specs"][0]["enabled"], false);
    assert!(built.notes.iter().all(|note| !note.contains("硬约束")));
}

#[test]
fn test_spec_for_an_indicator_the_contract_lacks_is_added_as_hard() {
    // 默认合同只有 S.
    let built = run(
        &storage(),
        with_specs(json!({"CSR": {"min": 62}, "Y": null, "S": {"max": 0.8}})),
    )
    .unwrap();
    assert_eq!(
        built.request["specs"],
        json!([
            {"indicator": "S", "direction": "Upper", "max": 0.8, "enabled": true},
            {"indicator": "CSR", "direction": "Lower", "min": 62.0, "enabled": true}
        ])
    );
    // 合同里本来就没有的指标写 null: 不新建, 但照样算本次提到过.
    assert_eq!(built.spec_changes, ["S", "Y", "CSR"]);
    assert_eq!(built.contract_name, "默认合同");
    // 只改合同, 煤池照旧.
    assert_eq!(pool_names(&built), ["甲", "乙"]);
    assert!(built.only.is_empty());
}

#[test]
fn test_bad_specs_are_refused_with_the_known_indicators() {
    let cases = [
        (json!({"硫": {"max": 1}}), "「硫」不是合同指标"),
        (json!({"s": {"max": 1}}), "「s」不是合同指标"),
        (json!({"S": {}}), "S: min 和 max 至少给一个"),
        (json!({"S": {"max": null}}), "S: min 和 max 至少给一个"),
        (json!({"S": {"max": -0.1}}), "S: max 须为不小于 0 的数"),
        (json!({"S": {"min": "0.5"}}), "S: min 须为不小于 0 的数"),
        (
            json!({"S": {"min": 1.2, "max": 0.8}}),
            "S: min 1.2 大于 max 0.8",
        ),
        (json!({"S": 0.8}), "S: 要么给 min / max, 要么写 null"),
    ];
    for (specs, expected) in cases {
        let error = run(&storage(), with_specs(specs.clone())).unwrap_err();
        assert!(error.contains(expected), "{specs}: {error}");
        assert!(error.contains("没有求解"), "{specs}: {error}");
        assert!(
            error.contains(
                "可用的指标: S (硫 S)、A (灰 A)、V (挥发 V)、G (粘结 G)、Y (胶质 Y)、\
                 petro (岩相 σ)、CSR (焦炭强度 CSR)、M (水分 M)"
            ),
            "{specs}: {error}"
        );
    }
    // 一项写错, 其余写对的也不生效.
    let error = run(
        &storage(),
        with_specs(json!({"S": {"max": 0.8}, "X": null, "A": {}})),
    )
    .unwrap_err();
    assert!(
        error.contains("「X」不是合同指标") && error.contains("A: min 和 max"),
        "{error}"
    );
    // 上下限相等是合法的区间.
    assert!(run(
        &storage(),
        with_specs(json!({"S": {"min": 0.8, "max": 0.8}}))
    )
    .is_ok());
}

#[test]
fn test_only_limits_the_pool_and_lets_switched_off_coals_in() {
    // 丙没启用、丁隐藏: 点了名就算进来; 启用着的乙没点到, 不参与.
    let built = run(&storage(), with_only(&["丁", " 丙", "甲", "丁"])).unwrap();
    assert_eq!(pool_names(&built), ["甲", "丙", "丁"]);
    // 候选煤按点名顺序, 用煤池里的写法, 重复点名只算一次.
    assert_eq!(built.only, ["丁", "丙", "甲"]);
    assert!(built.excluded.is_empty() && built.excluded_idle.is_empty());
    assert_eq!(built.request["coals"], json!([]));
    assert_eq!(built.request["price_factor"]["estimate_unpriced"], true);
    // 没点到的重名煤、数据非法的煤不再出现在提示里.
    assert!(built.notes.is_empty(), "{:?}", built.notes);
    // 合同不受影响.
    assert_eq!(
        built.request["specs"],
        master()["default_contract"]["specs"]
    );
}

#[test]
fn test_only_refuses_archived_duplicate_and_invalid_coals() {
    let error = run(
        &storage(),
        with_only(&["甲", "停了", "重名", "坏数", "没这煤"]),
    )
    .unwrap_err();
    for expected in [
        "「停了」这次用不了: 煤矿已停用",
        "「重名」这次用不了: 煤池里有重名的煤 (重名、重名 )",
        "「坏数」这次用不了: 数据非法",
        "没有叫「没这煤」的煤",
        "本次没有求解",
        "已停用、重名或数据非法的煤, 点了名这次也用不了",
    ] {
        assert!(error.contains(expected), "缺「{expected}」: {error}");
    }
    assert!(!error.contains("「甲」"), "对得上的煤不该点名: {error}");

    // 对不上时列出的相近煤名包括没启用的 (点了名就能用).
    let error = run(&storage(), with_only(&["甲", "名"])).unwrap_err();
    assert!(error.contains("名字里带「名」的有: 重名、重名 "), "{error}");
    let error = run(&storage(), with_only(&["甲", "  "])).unwrap_err();
    assert!(error.contains("煤名是空的"), "{error}");
}

#[test]
fn test_only_explains_a_repeated_bad_name_once() {
    // 对不上的、用不了的煤名重复写 (含只差空格与大小写的), 说明里各列一次, 同 exclude.
    let error = run(
        &storage(),
        with_only(&[
            "甲",
            "没这煤",
            "停了",
            "没这煤",
            " 没这煤",
            "停了 ",
            "",
            " ",
        ]),
    )
    .unwrap_err();
    for line in ["没有叫「没这煤」的煤", "「停了」这次用不了", "煤名是空的"]
    {
        assert_eq!(error.matches(line).count(), 1, "「{line}」: {error}");
    }
    assert!(!error.contains("「 没这煤」"), "{error}");
    // 结尾的两句规矩照旧各说一遍.
    assert!(
        error.contains("请给出完整煤名")
            && error.contains("已停用、重名或数据非法的煤, 点了名这次也用不了"),
        "{error}"
    );
    // 不同的错名字仍各列各的.
    let error = run(&storage(), with_only(&["甲", "没这煤", "也没这煤"])).unwrap_err();
    assert!(
        error.contains("没有叫「没这煤」的煤") && error.contains("没有叫「也没这煤」的煤"),
        "{error}"
    );
}

#[test]
fn test_only_then_exclude() {
    let options = TodayOptions {
        exclude: vec!["丙".into(), "乙".into(), "停了".into()],
        ..with_only(&["甲", "丙", "丁"])
    };
    let built = run(&storage(), options).unwrap();
    assert_eq!(pool_names(&built), ["甲", "丁"]);
    assert_eq!(built.only, ["甲", "丁"]);
    assert_eq!(built.excluded, ["丙"]);
    // 排除点到候选煤以外的煤: 如实说它本来就不在候选里.
    assert_eq!(
        built.excluded_idle,
        [
            ("乙".to_string(), "不在本次候选煤里"),
            ("停了".to_string(), "不在本次候选煤里")
        ]
    );
}

#[test]
fn test_too_few_candidates_is_a_clear_error() {
    let error = run(&storage(), with_only(&["甲"])).unwrap_err();
    assert!(
        error.contains("只剩 1 种") && error.contains("至少要 2 种"),
        "{error}"
    );
    let options = TodayOptions {
        exclude: vec!["丙".into()],
        ..with_only(&["甲", "丙"])
    };
    let error = run(&storage(), options).unwrap_err();
    assert!(error.contains("只剩 1 种"), "{error}");

    // 验算有 1 种就行, 一种不剩才报错.
    let options = TodayOptions {
        fixed_ratios: ratios(&[("丙", 1.0)]),
        ..with_only(&["丙"])
    };
    let built = run(&storage(), options).unwrap();
    assert_eq!(built.request["fixed_ratios"], json!({"丙": 1.0}));
    assert_eq!(built.only, ["丙"]);
    let options = TodayOptions {
        fixed_ratios: ratios(&[("丙", 1.0)]),
        exclude: vec!["丙".into()],
        ..with_only(&["丙"])
    };
    let error = run(&storage(), options).unwrap_err();
    assert!(error.contains("一种不剩, 没法验算"), "{error}");
}

#[test]
fn test_ratios_must_stay_inside_only() {
    // 给了 only: 没启用的丙、隐藏的丁都能验算, 煤名换成煤池里的写法.
    let options = TodayOptions {
        fixed_ratios: ratios(&[("丙 ", 2.0), ("丁", 1.0)]),
        ..with_only(&["甲", "丙", "丁"])
    };
    let built = run(&storage(), options).unwrap();
    assert_eq!(built.request["fixed_ratios"], json!({"丙": 2.0, "丁": 1.0}));
    assert_eq!(pool_names(&built), ["甲", "丙", "丁"]);

    // 配比点到 only 以外的煤 (哪怕它在煤池里启用着) 不行.
    let options = TodayOptions {
        fixed_ratios: ratios(&[("甲", 1.0), ("乙", 1.0)]),
        ..with_only(&["甲", "丙"])
    };
    let error = run(&storage(), options).unwrap_err();
    assert!(error.contains("「乙」不在本次候选煤 (only) 里"), "{error}");
    assert!(
        error.contains("给了 only 时, 验算配比里的煤都要列在 only 里"),
        "{error}"
    );
    assert!(!error.contains("不参与求解的煤"), "{error}");
}

#[test]
fn test_ratio_on_a_switched_off_coal_points_to_only() {
    const HINT: &str = "要把没启用或隐藏的煤算进来, 把这次要用的煤名都列进 only";
    let check = |pairs: &[(&str, f64)]| {
        let options = TodayOptions {
            fixed_ratios: ratios(pairs),
            ..Default::default()
        };
        run(&storage(), options).unwrap_err()
    };
    for name in ["丙", "丁"] {
        let error = check(&[("甲", 1.0), (name, 1.0)]);
        assert!(error.contains("在煤池里但不参与求解"), "{error}");
        assert!(error.contains(HINT), "{name}: {error}");
    }
    // 已停用、重名、数据非法的煤列进 only 也没用, 不给这个提示.
    for name in ["停了", "重名", "坏数"] {
        let error = check(&[("甲", 1.0), (name, 1.0)]);
        assert!(error.contains("在煤池里但不参与求解"), "{error}");
        assert!(!error.contains("only"), "{name}: {error}");
    }
    // 煤名对不上不是这回事.
    assert!(!check(&[("甲", 1.0), ("没这煤", 1.0)]).contains("only"));
}
