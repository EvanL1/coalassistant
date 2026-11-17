//! 行情与每日估价快照的排版: `market` 工具, 以及 `find_coal` 里带涨跌与一周后预测的那一行.
//! 预测只作参考展示, 这里照数据写出来, 不进配方 (与网页 `priceForecast.ts` 同一口径).
use chrono::NaiveDate;
use serde_json::{json, Value};

use super::{trim_num, Context};
use crate::today::ResolvedCoal;

const UNAVAILABLE: &str = "行情数据暂时取不到";

/// `market` 的结尾: 模型转述预测时会丢掉 "仅供参考", 还据此劝客户尽快锁价, 所以把转告的规矩写在数旁边.
const CAVEAT: &str = "预测按指数自身近期走势推算, 行情拐头时会滞后, 只作参考, 不进配方; \
                      转告客户时要说明这只是参考, 不要据此替客户拿买卖的主意。";

/// 指数值显示到几位小数; 涨跌按同样的位数算.
const INDEX_DIGITS: usize = 1;

/// 涨跌记号: ▲12 / ▼8 / 持平, 按显示出来的两个数算 (各自取到 `digits` 位小数再相减).
/// 读的人和模型会拿显示的数去核: 2056.6 → 2035.4 显示成 2057 与 2035, 涨跌得是 ▼22;
/// 先减后取整会写成 ▼21, 与显示的两个数对不上.
fn change_mark(now: f64, before: f64, digits: usize) -> String {
    let shown = |value: f64| trim_num(value, digits).parse::<f64>().unwrap_or(value);
    let change = shown(now) - shown(before);
    let size = trim_num(change.abs(), digits);
    if size == "0" {
        return "持平".into();
    }
    let arrow = if change > 0.0 { "▲" } else { "▼" };
    format!("{arrow}{size}")
}

/// `find_coal` 里的一行: 共享煤库每日估价快照的价、较前一日的涨跌、一周后的预测价.
/// 没有快照、或快照里没有这种煤 (用户自建的煤) 为 None.
pub(super) fn snapshot_line(coal: &ResolvedCoal, context: &Context) -> Option<String> {
    let snapshot = context.coal_prices.as_ref()?;
    let day = snapshot.day?;
    let entry = snapshot
        .prices
        .iter()
        .find(|entry| entry.name == coal.name)?;
    let mut line = format!("  每日估价快照 ({day}): {} 元/吨", trim_num(entry.price, 0));
    if let Some(prev) = entry.prev_price {
        line.push_str(&format!(", 较前一日 {}", change_mark(entry.price, prev, 0)));
    }
    // 预测的那天还没到才算数: 快照停更后, 旧预测不再显示 (同网页 `isAhead`).
    let ahead = snapshot.forecast_day.filter(|date| *date > context.today);
    if let (Some(date), Some(price)) = (ahead, entry.forecast_price) {
        line.push_str(&format!(
            "; 预测一周后 ({date}) 约 {} 元/吨, 较快照 {} (按指数近期走势推算, 仅供参考)",
            trim_num(price, 0),
            change_mark(price, entry.price, 0)
        ));
    }
    if !coal.overridden.is_empty() {
        line.push_str("; 快照按共享煤库的指标算, 不含你改过的值");
    }
    Some(line)
}

/// `tools/list` 里 `market` 的说明与参数.
pub(super) fn tool() -> (&'static str, Value) {
    (
        "炼焦煤行情: 钢铁网炼焦煤价格指数各品种 (综合指数在最前) 的最新值、较上一期的涨跌与一周后的预测。预测只作参考, 不进配方、不当报价。",
        json!({ "type": "object", "properties": {} }),
    )
}

/// `market` 工具: 钢铁网炼焦煤价格指数各序列的最新值、较上一期的涨跌与一周后的预测.
pub(super) fn market_text(context: &Context) -> Result<String, String> {
    let mut series: Vec<&Value> = context.market.iter().flatten().collect();
    // 综合指数 (只展示的序列) 在最前, 其余照原顺序.
    series.sort_by_key(|row| row["display_only"] != true);
    let latest = series
        .iter()
        .filter_map(|row| latest_point(row))
        .map(|(date, _)| date)
        .max()
        .ok_or(UNAVAILABLE)?;
    let mut out = vec![format!(
        "钢铁网炼焦煤价格指数 (标准品出厂含税价, 元/吨), 最新一期 {latest}:"
    )];
    out.extend(series.iter().map(|row| series_line(row, context.today)));
    out.push(CAVEAT.into());
    Ok(out.join("\n"))
}

fn point(value: &Value) -> Option<(&str, f64)> {
    Some((value["date"].as_str()?, value["price"].as_f64()?))
}

/// 序列最新一期的 (日期, 值); `points` 按日期升序.
fn latest_point(row: &Value) -> Option<(&str, f64)> {
    row["points"].as_array()?.last().and_then(point)
}

/// "- 低硫主焦煤: 2044 (2026-11-13), 较上一期 ▲5; 预测一周后 (2026-11-20) 约 2050, ▲6".
fn series_line(row: &Value, today: NaiveDate) -> String {
    let name = row["type_name"]
        .as_str()
        .or(row["name"].as_str())
        .unwrap_or("未命名序列");
    let Some((date, price)) = latest_point(row) else {
        return format!("- {name}: 还没有数据");
    };
    let mut line = format!("- {name}: {} ({date})", trim_num(price, INDEX_DIGITS));
    let previous = row["points"]
        .as_array()
        .and_then(|points| points.iter().rev().nth(1))
        .and_then(point);
    if let Some((_, previous)) = previous {
        line.push_str(&format!(
            ", 较上一期 {}",
            change_mark(price, previous, INDEX_DIGITS)
        ));
    }
    let forecast = point(&row["forecast"])
        .filter(|(date, _)| date.parse::<NaiveDate>().is_ok_and(|date| date > today));
    if let Some((date, value)) = forecast {
        line.push_str(&format!(
            "; 预测一周后 ({date}) 约 {}, {}",
            trim_num(value, INDEX_DIGITS),
            change_mark(value, price, INDEX_DIGITS)
        ));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coal_prices::{CoalPriceEntry, CoalPriceSnapshot};
    use crate::database::UserStorage;
    use serde_json::json;

    fn context(prefs: Value) -> Context {
        Context {
            master: json!({
                "default_contract": {"name": "默认合同", "specs": []},
                "coals": [
                    {"name": "甲煤", "status": "active", "props": {"S": 0.6}},
                    {"name": "乙煤", "status": "active", "props": {"S": 1.2}}
                ]
            }),
            storage: UserStorage {
                initialized: true,
                coal_prefs: prefs,
                contract: None,
                quantity: 1000.0,
                user_coals: json!([]),
                shovel_tons: None,
            },
            benchmarks: Vec::new(),
            rank_k: None,
            coal_prices: None,
            market: None,
            today: "2026-11-14".parse().unwrap(),
        }
    }

    fn entry(prev: Option<f64>, forecast: Option<f64>) -> CoalPriceEntry {
        CoalPriceEntry {
            name: "甲煤".into(),
            price: 2240.0,
            prev_price: prev,
            forecast_price: forecast,
            points: 1010.0,
            benchmark: None,
            missing: Vec::new(),
        }
    }

    fn snapshot(entry: CoalPriceEntry, forecast_day: Option<&str>) -> CoalPriceSnapshot {
        CoalPriceSnapshot {
            ok: true,
            day: Some("2026-11-14".parse().unwrap()),
            prev_day: Some("2026-11-13".parse().unwrap()),
            forecast_day: forecast_day.map(|day| day.parse().unwrap()),
            computed_at: chrono::Utc::now(),
            prices: vec![entry],
        }
    }

    fn find(context: &Context, name: &str) -> String {
        let args = json!({ "name": name });
        super::super::find_coal(args.as_object().unwrap(), context).unwrap()
    }

    #[test]
    fn test_change_mark_is_the_difference_of_the_shown_numbers() {
        assert_eq!(change_mark(2240.0, 2228.0, 0), "▲12");
        assert_eq!(change_mark(2232.0, 2240.0, 0), "▼8");
        // 2057 与 2035: 差是 22, 不是原值之差 21.2 取整出来的 21.
        assert_eq!(change_mark(2035.4, 2056.6, 0), "▼22");
        assert_eq!(change_mark(2056.6, 2035.4, 0), "▲22");
        // 显示成同一个数就是持平, 哪怕原值差了快 1 元; 显示的数不同就不是持平.
        assert_eq!(change_mark(2240.4, 2239.6, 0), "持平");
        assert_eq!(change_mark(2240.0, 2240.0, 0), "持平");
        assert_eq!(change_mark(2240.6, 2240.4, 0), "▲1");
        // 指数值保留 1 位小数: 涨跌也到 1 位, 末尾的 .0 去掉.
        assert_eq!(change_mark(1888.5, 1890.0, 1), "▼1.5");
        assert_eq!(change_mark(2044.0, 2039.0, 1), "▲5");
        assert_eq!(change_mark(1888.46, 1890.04, 1), "▼1.5");
        assert_eq!(change_mark(1700.04, 1699.96, 1), "持平");
        assert_eq!(change_mark(2044.3, 2039.1, 1), "▲5.2");
    }

    #[test]
    fn test_snapshot_changes_match_the_shown_prices() {
        let mut context = context(json!({}));
        let mut entry = entry(Some(2035.4), Some(2035.4));
        entry.price = 2056.6;
        context.coal_prices = Some(snapshot(entry, Some("2026-11-20")));
        let text = find(&context, "甲煤");
        assert!(
            text.contains(
                "每日估价快照 (2026-11-14): 2057 元/吨, 较前一日 ▲22; \
                 预测一周后 (2026-11-20) 约 2035 元/吨, 较快照 ▼22 (按指数近期走势推算, 仅供参考)"
            ),
            "{text}"
        );
    }

    #[test]
    fn test_find_coal_shows_snapshot_change_and_forecast() {
        let mut context = context(json!({}));
        context.coal_prices = Some(snapshot(
            entry(Some(2228.0), Some(2232.0)),
            Some("2026-11-20"),
        ));
        let text = find(&context, "甲煤");
        assert!(
            text.contains(
                "\n  每日估价快照 (2026-11-14): 2240 元/吨, 较前一日 ▲12; \
                 预测一周后 (2026-11-20) 约 2232 元/吨, 较快照 ▼8 (按指数近期走势推算, 仅供参考)"
            ),
            "{text}"
        );
        assert!(!text.contains("你改过的值"), "{text}");
        // 快照里没有的煤不带这一行.
        assert!(!find(&context, "乙煤").contains("每日估价快照"));
    }

    #[test]
    fn test_snapshot_line_without_previous_day_or_forecast() {
        let mut context = context(json!({}));
        context.coal_prices = Some(snapshot(entry(None, None), None));
        let text = find(&context, "甲煤");
        assert!(
            text.contains("\n  每日估价快照 (2026-11-14): 2240 元/吨"),
            "{text}"
        );
        assert!(
            !text.contains("较前一日") && !text.contains("预测"),
            "{text}"
        );

        context.coal_prices = Some(snapshot(entry(Some(2240.2), None), None));
        assert!(
            find(&context, "甲煤").contains("2240 元/吨, 较前一日 持平"),
            "两天显示成同一个数, 写持平"
        );
    }

    #[test]
    fn test_forecast_is_hidden_once_its_day_has_come() {
        let mut context = context(json!({}));
        context.coal_prices = Some(snapshot(
            entry(Some(2228.0), Some(2232.0)),
            Some("2026-11-20"),
        ));
        context.today = "2026-11-20".parse().unwrap();
        let text = find(&context, "甲煤");
        assert!(text.contains("较前一日 ▲12"), "{text}");
        assert!(!text.contains("预测"), "预测的那天已到, 不再显示: {text}");

        // 有预测日、这种煤却没有预测价: 同样不写.
        context.today = "2026-11-14".parse().unwrap();
        context.coal_prices = Some(snapshot(entry(None, None), Some("2026-11-20")));
        assert!(!find(&context, "甲煤").contains("预测"));
    }

    #[test]
    fn test_snapshot_line_flags_user_overrides() {
        let mut context = context(json!({"甲煤": {"props_override": {"S": 0.9}}}));
        context.coal_prices = Some(snapshot(
            entry(Some(2228.0), Some(2232.0)),
            Some("2026-11-20"),
        ));
        let text = find(&context, "甲煤");
        assert!(
            text.contains("仅供参考); 快照按共享煤库的指标算, 不含你改过的值"),
            "{text}"
        );
        // 改回与煤库相同的值不算改过.
        context.storage.coal_prefs = json!({"甲煤": {"props_override": {"S": 0.6}}});
        assert!(!find(&context, "甲煤").contains("你改过的值"));
    }

    #[test]
    fn test_find_coal_is_unchanged_without_a_snapshot() {
        let mut bare = context(json!({}));
        let before = find(&bare, "甲煤");
        // 还没有任何快照 (day 为空) 与读不到快照一样, 不多一行.
        bare.coal_prices = Some(CoalPriceSnapshot {
            ok: true,
            day: None,
            prev_day: None,
            forecast_day: None,
            computed_at: chrono::Utc::now(),
            prices: Vec::new(),
        });
        assert_eq!(find(&bare, "甲煤"), before);
        assert!(!before.contains("每日估价快照"));
    }

    fn series(name: &str, points: &[(&str, f64)], forecast: Option<(&str, f64)>) -> Value {
        let mut row = json!({
            "code": name, "type_name": name, "name": format!("{name}指数"),
            "points": points.iter().map(|(date, price)| json!({"date": date, "price": price})).collect::<Vec<_>>(),
        });
        if let Some((date, price)) = forecast {
            row["forecast"] = json!({"date": date, "price": price});
        }
        row
    }

    #[test]
    fn test_market_lists_series_with_change_and_forecast() {
        let mut context = context(json!({}));
        let mut composite = series(
            "综合炼焦煤",
            &[("2026-11-12", 1890.0), ("2026-11-13", 1888.5)],
            None,
        );
        composite["display_only"] = json!(true);
        context.market = Some(vec![
            series(
                "低硫主焦煤",
                &[("2026-11-12", 2039.0), ("2026-11-13", 2044.0)],
                Some(("2026-11-20", 2050.0)),
            ),
            composite,
            series("肥煤", &[("2026-11-13", 1700.0)], None),
            series("气煤", &[], None),
        ]);
        let text = market_text(&context).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "钢铁网炼焦煤价格指数 (标准品出厂含税价, 元/吨), 最新一期 2026-11-13:",
                "- 综合炼焦煤: 1888.5 (2026-11-13), 较上一期 ▼1.5",
                "- 低硫主焦煤: 2044 (2026-11-13), 较上一期 ▲5; 预测一周后 (2026-11-20) 约 2050, ▲6",
                "- 肥煤: 1700 (2026-11-13)",
                "- 气煤: 还没有数据",
                "预测按指数自身近期走势推算, 行情拐头时会滞后, 只作参考, 不进配方; \
                 转告客户时要说明这只是参考, 不要据此替客户拿买卖的主意。",
            ]
        );
    }

    #[test]
    fn test_market_changes_match_the_shown_values() {
        let mut context = context(json!({}));
        context.market = Some(vec![series(
            "低硫主焦煤",
            &[("2026-11-12", 2039.96), ("2026-11-13", 2044.26)],
            Some(("2026-11-20", 2041.74)),
        )]);
        let text = market_text(&context).unwrap();
        // 显示 2040 → 2044.3 → 2041.7: 涨跌是显示值之差 4.3 与 2.6, 不是原值之差取整的 4 与 3.
        assert!(
            text.contains(
                "- 低硫主焦煤: 2044.3 (2026-11-13), 较上一期 ▲4.3; \
                 预测一周后 (2026-11-20) 约 2041.7, ▼2.6"
            ),
            "{text}"
        );
    }

    #[test]
    fn test_market_drops_a_forecast_whose_day_has_come() {
        let mut context = context(json!({}));
        context.market = Some(vec![series(
            "低硫主焦煤",
            &[("2026-11-12", 2039.0), ("2026-11-13", 2044.0)],
            Some(("2026-11-14", 2050.0)),
        )]);
        let text = market_text(&context).unwrap();
        assert!(text.contains("较上一期 ▲5"), "{text}");
        assert!(!text.contains("预测一周后"), "{text}");
    }

    #[test]
    fn test_market_without_data_is_an_error() {
        let mut context = context(json!({}));
        assert_eq!(market_text(&context).unwrap_err(), UNAVAILABLE);
        // 序列配置在, 库里一期都没有: 同样算取不到.
        context.market = Some(vec![series("肥煤", &[], None)]);
        assert_eq!(market_text(&context).unwrap_err(), UNAVAILABLE);

        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                             "params": {"name": "market", "arguments": {}}});
        let reply = super::super::handle(&message, Some(&context)).unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(reply["result"]["content"][0]["text"], UNAVAILABLE);
    }
}
