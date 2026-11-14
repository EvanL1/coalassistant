//! 一周后的价格预测: 钢铁网炼焦煤指数各序列 5 个交易日后的值.
//!
//! 模型: 未来 5 个交易日的对数涨幅 = 截距 + 自己过去 1/5/20 日涨幅 + 综合指数过去 1/5 日涨幅
//! 的线性组合, 用这条序列的全部历史做最小二乘, 每次现拟合 (几千行 × 6 个系数, 不值得存).
//! 综合指数自己、或综合指数没跟上最新一天时, 只用自己那三项.
//!
//! 为什么是它: 在 2018-04 起的日度历史上逐日滚动回测 (research/price_forecast/backtest.py),
//! 一周期限上它的平均误差比 "一周后 = 今天" 低一到两成, 比 ARIMA、指数平滑稳; 只用近一年拟合
//! 反而不如全历史, 所以历史不够长就不给预测 ([`MIN_SAMPLES`]). 分指数常常几天不动, 综合指数先
//! 反映了别的品种的新成交, 所以带上它. 20 个交易日以上没有可预测性, 不做.
//!
//! 局限: 指数是慢慢走到位的, 模型本质是顺势外推, 拐点会滞后. 只在界面上当参考, 不进求解.
//!
//! 每小时随估价快照算一次 ([`refresh`]), 存表 `mcci_forecast` (每个 "所依据的最新一期" 一条,
//! 留作日后对照实际值); 煤的预测价由 `coal_prices` 按预测的指数重估.

use std::collections::HashMap;

use chrono::{Days, NaiveDate};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::coal_index::IndexPoint;
use crate::mcci;

/// 预测多远: 5 个交易日, 即一周.
const HORIZON: usize = 5;
/// 自己过去这几个交易日的涨幅.
const OWN_LAGS: [usize; 3] = [1, 5, 20];
/// 综合指数过去这几个交易日的涨幅.
const MARKET_LAGS: [usize; 2] = [1, 5];
const MAX_LAG: usize = 20;
/// 至少这么多个训练样本 (约两年) 才给预测: 回测里只用一年历史拟合不比 "不变" 准.
const MIN_SAMPLES: usize = 500;
/// 5 个交易日后落在哪天: 按日历一周算 (遇节假日实际会晚几天, 界面只说 "一周后").
const TARGET_AFTER_DAYS: u64 = 7;
/// 正规方程对角线上加的一点点: 序列长期不动时各项涨幅全为 0, 方程奇异.
const RIDGE: f64 = 1e-10;

/// 一条序列的预测: 依据到 `made_on` 这一期 (值 `base_price`), 预测 `date` 那天是 `price`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Forecast {
    pub made_on: String,
    pub base_price: f64,
    pub date: String,
    pub price: f64,
}

/// 一行特征: 截距、自己各期涨幅、综合指数各期涨幅 (对数价之差).
fn features(own: &[f64], market: Option<&[f64]>, at: usize) -> Vec<f64> {
    let change = |logs: &[f64], lag: &usize| logs[at] - logs[at - lag];
    let own_part = OWN_LAGS.iter().map(|lag| change(own, lag));
    let market_part = market
        .into_iter()
        .flat_map(|market| MARKET_LAGS.iter().map(move |lag| change(market, lag)));
    std::iter::once(1.0)
        .chain(own_part)
        .chain(market_part)
        .collect()
}

/// 最小二乘系数: 正规方程 + 列主元消去. 方程解不出 (数值上奇异) 返回 None.
fn least_squares(rows: &[(Vec<f64>, f64)]) -> Option<Vec<f64>> {
    let size = rows.first()?.0.len();
    // 增广矩阵 [X'X + λI | X'y].
    let mut matrix = vec![vec![0.0; size + 1]; size];
    for (x, y) in rows {
        for i in 0..size {
            for j in 0..size {
                matrix[i][j] += x[i] * x[j];
            }
            matrix[i][size] += x[i] * y;
        }
    }
    for (i, row) in matrix.iter_mut().enumerate() {
        row[i] += RIDGE;
    }
    for col in 0..size {
        let pivot =
            (col..size).max_by(|a, b| matrix[*a][col].abs().total_cmp(&matrix[*b][col].abs()))?;
        if matrix[pivot][col].abs() < f64::MIN_POSITIVE {
            return None;
        }
        matrix.swap(col, pivot);
        let pivot_row = matrix[col].clone();
        for (row, values) in matrix.iter_mut().enumerate() {
            if row == col {
                continue;
            }
            let factor = values[col] / pivot_row[col];
            for (value, pivot_value) in values.iter_mut().zip(&pivot_row).skip(col) {
                *value -= factor * pivot_value;
            }
        }
    }
    Some((0..size).map(|i| matrix[i][size] / matrix[i][i]).collect())
}

/// 未来 [`HORIZON`] 个交易日的对数涨幅. `own` 与 `market` 是按同一批交易日对齐的对数价 (升序).
fn expected_change(own: &[f64], market: Option<&[f64]>) -> Option<f64> {
    let total = own.len();
    if total < MAX_LAG + HORIZON + MIN_SAMPLES {
        return None;
    }
    let rows: Vec<(Vec<f64>, f64)> = (MAX_LAG..total - HORIZON)
        .map(|at| (features(own, market, at), own[at + HORIZON] - own[at]))
        .collect();
    let beta = least_squares(&rows)?;
    let change: f64 = features(own, market, total - 1)
        .iter()
        .zip(&beta)
        .map(|(x, b)| x * b)
        .sum();
    change.is_finite().then_some(change)
}

/// 两条序列都有值的交易日上的对数价 (自己的, 综合指数的).
fn aligned(own: &[IndexPoint], market: &[IndexPoint]) -> (Vec<f64>, Vec<f64>) {
    let by_date: HashMap<&str, f64> = market
        .iter()
        .map(|point| (point.date.as_str(), point.price))
        .collect();
    own.iter()
        .filter_map(|point| {
            let market_price = by_date.get(point.date.as_str())?;
            Some((point.price.ln(), market_price.ln()))
        })
        .unzip()
}

/// 预测一条序列 (升序, 值都为正). `market` 是综合指数; 它没跟上这条序列的最新一天、或对齐后
/// 历史不够长时, 退回只用自己. 历史不够长返回 None.
pub(crate) fn predict(own: &[IndexPoint], market: Option<&[IndexPoint]>) -> Option<Forecast> {
    let latest = own.last()?;
    let with_market = market
        .filter(|market| market.last().is_some_and(|point| point.date == latest.date))
        .and_then(|market| {
            let (own_logs, market_logs) = aligned(own, market);
            expected_change(&own_logs, Some(&market_logs))
        });
    let change = with_market.or_else(|| {
        let logs: Vec<f64> = own.iter().map(|point| point.price.ln()).collect();
        expected_change(&logs, None)
    })?;
    let target = latest.date.parse::<NaiveDate>().ok()? + Days::new(TARGET_AFTER_DAYS);
    Some(Forecast {
        made_on: latest.date.clone(),
        base_price: latest.price,
        date: target.format("%Y-%m-%d").to_string(),
        price: (latest.price * change.exp() * 10.0).round() / 10.0,
    })
}

/// 全部序列的预测, 按序列代码. `config` 是 [`mcci::series_config`]: 标了 `display_only` 的第一条
/// 就是综合指数. 预测的那天已经不在 `today` 之后 (指数停更一周以上) 的不要.
pub(crate) fn for_all(
    config: &[Value],
    history: &HashMap<String, Vec<IndexPoint>>,
    today: NaiveDate,
) -> HashMap<String, Forecast> {
    let market_code = config
        .iter()
        .find(|series| series["display_only"] == true)
        .and_then(|series| series["code"].as_str());
    let market = market_code.and_then(|code| history.get(code));
    let today = today.format("%Y-%m-%d").to_string();
    config
        .iter()
        .filter_map(|series| {
            let code = series["code"].as_str()?;
            let market = market
                .filter(|_| Some(code) != market_code)
                .map(Vec::as_slice);
            let forecast = predict(history.get(code)?, market)?;
            (forecast.date > today).then(|| (code.to_string(), forecast))
        })
        .collect()
}

/// 存各序列的预测: 同一期 (`made_on`) 重算就覆盖.
pub(crate) async fn store(
    pool: &PgPool,
    forecasts: &HashMap<String, Forecast>,
) -> Result<(), sqlx::Error> {
    let parse = |date: &str| date.parse::<NaiveDate>().ok();
    let rows: Vec<(&str, NaiveDate, NaiveDate, &Forecast)> = forecasts
        .iter()
        .filter_map(|(code, forecast)| {
            Some((
                code.as_str(),
                parse(&forecast.made_on)?,
                parse(&forecast.date)?,
                forecast,
            ))
        })
        .collect();
    let codes: Vec<&str> = rows.iter().map(|row| row.0).collect();
    let made_on: Vec<NaiveDate> = rows.iter().map(|row| row.1).collect();
    let targets: Vec<NaiveDate> = rows.iter().map(|row| row.2).collect();
    let base: Vec<f64> = rows.iter().map(|row| row.3.base_price).collect();
    let values: Vec<f64> = rows.iter().map(|row| row.3.price).collect();
    sqlx::query(
        r#"
        INSERT INTO mcci_forecast (series_code, made_on, base_value, target_date, value)
        SELECT * FROM UNNEST($1::text[], $2::date[], $3::float8[], $4::date[], $5::float8[])
        ON CONFLICT (series_code, made_on) DO UPDATE SET
            base_value = EXCLUDED.base_value,
            target_date = EXCLUDED.target_date,
            value = EXCLUDED.value,
            computed_at = NOW()
        "#,
    )
    .bind(&codes)
    .bind(&made_on)
    .bind(&base)
    .bind(&targets)
    .bind(&values)
    .execute(pool)
    .await?;
    Ok(())
}

/// 按库里的全部指数历史重算各序列的预测并存表. 读不到历史返回错误; 存表失败只记日志
/// (预测照样返回, 不耽误当轮的煤价预测).
pub(crate) async fn refresh(
    pool: &PgPool,
    master: &Value,
    today: NaiveDate,
) -> Result<HashMap<String, Forecast>, sqlx::Error> {
    let history = mcci::full_history(pool).await?;
    let forecasts = for_all(&mcci::series_config(master), &history, today);
    if let Err(error) = store(pool, &forecasts).await {
        tracing::warn!(%error, "指数预测存表失败, 行情条暂时看不到预测");
    }
    Ok(forecasts)
}

/// 每条序列最近一次的预测, 只留预测的那天还在 `today` 之后的.
pub(crate) async fn latest(
    pool: &PgPool,
    today: NaiveDate,
) -> Result<HashMap<String, Forecast>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (series_code) series_code, made_on, base_value, target_date, value
        FROM mcci_forecast
        ORDER BY series_code, made_on DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let day = |date: NaiveDate| date.format("%Y-%m-%d").to_string();
    let mut forecasts = HashMap::new();
    for row in rows {
        let target: NaiveDate = row.try_get("target_date")?;
        if target <= today {
            continue;
        }
        forecasts.insert(
            row.try_get("series_code")?,
            Forecast {
                made_on: day(row.try_get("made_on")?),
                base_price: row.try_get("base_value")?,
                date: day(target),
                price: row.try_get("value")?,
            },
        );
    }
    Ok(forecasts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 从 2020-01-01 起每天一个点 (测试里把日历天当交易日用).
    fn series(prices: impl IntoIterator<Item = f64>) -> Vec<IndexPoint> {
        let start: NaiveDate = "2020-01-01".parse().unwrap();
        prices
            .into_iter()
            .enumerate()
            .map(|(i, price)| IndexPoint {
                date: (start + Days::new(i as u64)).format("%Y-%m-%d").to_string(),
                price,
            })
            .collect()
    }

    const ENOUGH: usize = MAX_LAG + HORIZON + MIN_SAMPLES;

    /// 确定性的伪随机涨幅 (线性同余), 落在 ±scale 之间.
    fn noise(seed: u64, scale: f64) -> impl Iterator<Item = f64> {
        let mut state = seed;
        std::iter::repeat_with(move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * scale
        })
    }

    fn day(date: &str) -> NaiveDate {
        date.parse().unwrap()
    }

    #[test]
    fn test_predict_needs_long_history() {
        let short = series((0..ENOUGH - 1).map(|i| 1000.0 + i as f64));
        assert_eq!(predict(&short, None), None);
        assert_eq!(predict(&[], None), None);
        assert!(predict(&series((0..ENOUGH).map(|i| 1000.0 + i as f64)), None).is_some());
    }

    #[test]
    fn test_predict_flat_series_stays_flat_a_week_later() {
        let flat = series(std::iter::repeat_n(1500.0, ENOUGH));
        let forecast = predict(&flat, None).unwrap();
        assert_eq!(forecast.price, 1500.0);
        assert_eq!(forecast.base_price, 1500.0);
        let latest: NaiveDate = flat.last().unwrap().date.parse().unwrap();
        assert_eq!(forecast.made_on, flat.last().unwrap().date);
        assert_eq!(
            forecast.date,
            (latest + Days::new(7)).format("%Y-%m-%d").to_string()
        );
    }

    #[test]
    fn test_predict_extends_a_steady_trend() {
        // 每天涨 0.1%: 5 个交易日后应是 ×1.001^5 (各项涨幅与截距共线, 也得解得出来).
        let growing = series((0..ENOUGH).map(|i| 1000.0 * 1.001f64.powi(i as i32)));
        let latest = growing.last().unwrap().price;
        let forecast = predict(&growing, None).unwrap();
        let expected = latest * 1.001f64.powi(5);
        assert!(
            (forecast.price - expected).abs() < 0.11,
            "{} vs {expected}",
            forecast.price
        );
    }

    #[test]
    fn test_predict_learns_momentum_from_history() {
        // 涨跌有惯性: 每天的涨幅 = 0.6 × 昨天的 + 噪声. 最后一天大涨, 预测应继续涨, 且涨幅接近
        // 0.6 + 0.6² + … + 0.6⁵ ≈ 1.38 倍的当日涨幅.
        let mut change = 0.0;
        let mut log = 7.0f64;
        let mut prices: Vec<f64> = noise(7, 0.004)
            .take(ENOUGH + 500)
            .map(|shock| {
                change = 0.6 * change + shock;
                log += change;
                log.exp()
            })
            .collect();
        let jump = 0.02;
        change = 0.6 * change + jump;
        prices.push((log + change).exp());
        let forecast = predict(&series(prices), None).unwrap();
        let predicted = (forecast.price / forecast.base_price).ln();
        let implied = change * (1..=5).map(|k| 0.6f64.powi(k)).sum::<f64>();
        assert!(predicted > 0.0);
        assert!(
            (predicted - implied).abs() < implied * 0.35,
            "{predicted} vs {implied}"
        );
    }

    #[test]
    fn test_predict_uses_market_only_when_it_has_the_latest_day() {
        // 自己一直不动, 综合指数领先一天: 综合指数昨天的涨幅就是自己今天的涨幅.
        let market_changes: Vec<f64> = noise(11, 0.01).take(ENOUGH + 200).collect();
        let mut market_log = 7.0;
        let mut own_log = 7.0;
        let mut market = Vec::new();
        let mut own = Vec::new();
        for (i, change) in market_changes.iter().enumerate() {
            market_log += change;
            own_log += if i == 0 { 0.0 } else { market_changes[i - 1] };
            market.push(market_log.exp());
            own.push(own_log.exp());
        }
        let (own, market) = (series(own), series(market));
        let alone = predict(&own, None).unwrap();
        let led = predict(&own, Some(&market)).unwrap();
        // 明天的涨幅 = 综合指数今天的涨幅: 带上综合指数才看得见.
        let tomorrow = *market_changes.last().unwrap();
        let error =
            |forecast: &Forecast| ((forecast.price / forecast.base_price).ln() - tomorrow).abs();
        assert!(
            error(&led) < error(&alone) * 0.5,
            "{} vs {}",
            error(&led),
            error(&alone)
        );
        // 综合指数少了最新一天: 当它没有, 结果与只用自己一样.
        let stale = &market[..market.len() - 1];
        assert_eq!(predict(&own, Some(stale)), Some(alone));
    }

    #[test]
    fn test_for_all_uses_display_series_as_market_and_drops_stale_forecasts() {
        let config = vec![
            json!({"code": "ZH", "display_only": true}),
            json!({"code": "A"}),
            json!({"code": "SHORT"}),
            json!({"code": "MISSING"}),
        ];
        let prices = || (0..ENOUGH).map(|i| 1000.0 + (i % 7) as f64);
        let history = HashMap::from([
            ("ZH".to_string(), series(prices())),
            ("A".to_string(), series(prices())),
            ("SHORT".to_string(), series(prices().take(100))),
        ]);
        let latest = day(&history["A"].last().unwrap().date);
        let forecasts = for_all(&config, &history, latest);
        let mut codes: Vec<&str> = forecasts.keys().map(String::as_str).collect();
        codes.sort_unstable();
        assert_eq!(codes, ["A", "ZH"]);
        // 综合指数自己不拿自己当市场: 与单独预测一致.
        assert_eq!(forecasts["ZH"], predict(&history["ZH"], None).unwrap());
        assert_eq!(
            forecasts["A"],
            predict(&history["A"], Some(&history["ZH"])).unwrap()
        );
        // 指数停更满一周: 预测的那天已经到了, 不再给.
        assert_eq!(for_all(&config, &history, latest + Days::new(6)).len(), 2);
        assert!(for_all(&config, &history, latest + Days::new(7)).is_empty());
    }
}
