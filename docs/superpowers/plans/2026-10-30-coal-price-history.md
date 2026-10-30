# 历史估价回算与走势图实施计划

> **For agentic workers:** 使用 superpowers:executing-plans 在本工作区逐个任务实施; 每个任务严格 TDD (先写失败测试并跑出 RED,
> 再实现到 GREEN)。用户已批准设计, 不再请求设计批准。步骤用 `- [ ]` 勾选记录。

**Goal:** 把共享煤库每种煤过去一年的估价按"当天能拿到的指数与报价"补算进 `coal_price_daily` (只补空, 不覆盖), 并在煤池卡片上点估价行看一年走势。

**Architecture:** 把"当前生效基准"拆成"一次读齐输入"+ 纯函数 `as_of(master, inputs, day)`, 实时快照与补算共用它; 补算是"算出缺的 (日期, 煤) → `ON CONFLICT DO NOTHING` 批量写"的后台任务, 挂在现有每小时循环里每天跑一次。前端把指数弹层里的图与弹层外壳抽成共用组件, 新增煤价弹层。

**Tech Stack:** Rust / Axum 0.8 / SQLx / PostgreSQL; React 19 / TypeScript / Vitest。

**Spec:** `docs/superpowers/specs/2026-10-30-coal-price-history-design.md` (先通读)

---

## 全局约束

- 在分支 `feat/coal-price-history` 的当前工作区实施; **不 commit、不 push**; 不动未跟踪的 `research/`。
- 不改核心 `blend_kit_rs` 与 master 数据 (`coal_master.json`); Rust 测试只用 fixture, 不断言 master 内容。
- 注释、日志、界面文案用中文, 标点跟随所在文件 (Rust 注释多为半角标点)。
- 每个 Rust 任务后跑 `cd blend_kit_server && cargo test --locked <过滤词>`; 每个前端任务后跑
  `cd doudou_blend && npx vitest run <文件>`。
- 现有测试一律不改期望 (指数弹层的测试在 `doudou_blend/src/IndexTicker.test.tsx` 的「指数走势弹层」一节, 必须原样通过)。

## 审查重点 (实施时自查, 交付后我会逐条审)

1. `as_of(D)` 绝不使用晚于 D 的报价、指数点、CCP 点; 推算比例只推到 D。
2. 补算绝不写今天, 绝不覆盖已有行 (只 `ON CONFLICT (day, name) DO NOTHING`)。
3. 补算的输入读取有任何失败 (库表、指数、CCP 取不到) 就**整轮不写**、下小时重试 —— 写下去就永久定格了, 不能拿缺料的结果填。
4. `current_from_pool` 对现有调用方 (`/api/price`、求解、实时快照) 行为不变 (唯一例外: 报价日在未来的补录报价到那天才生效)。
5. 同名同日的库表报价仍是 `created_at` 最新的胜出。
6. 指数弹层外观与测试不变; 卡片上点估价行不触发卡片自身的点击 (打开编辑器)。
7. 某天 `as_of` 为空时必须跳过, 不能交给 `snapshot_rows` (`fill_price_request` 遇空基准会退回 master 自带的、含未来日期的报价)。

## 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `blend_kit_server/src/benchmarks.rs` | 改 | `BenchmarkInputs` / `load_inputs` / `as_of`; `current_from_pool` 变薄包装 |
| `blend_kit_server/src/mcci.rs` | 改 | `HISTORY_DAYS` 改 `pub(crate)` |
| `blend_kit_server/migrations/0012_coal_price_source.sql` | 新 | `source` 列 |
| `blend_kit_server/src/coal_prices.rs` | 改 | 补算 (纯函数 + IO)、调度、历史接口与 DTO |
| `blend_kit_server/src/lib.rs` | 改 | 路由 `/api/coal-prices/history` + handler 测试 |
| `scripts/check_repository_consistency.mjs` | 改 | 登记新 DTO |
| `doudou_blend/src/types.ts` | 改 | `CoalPricePoint` / `CoalPriceHistory` |
| `doudou_blend/src/coalPrices.ts` (+test) | 改 | `fetchCoalPriceHistory`、导出 `isModified`、`hasPriceHistory` |
| `doudou_blend/src/mcci.ts` | 改 | `lastDays` 泛型化 |
| `doudou_blend/src/PriceChart.tsx` (+test) | 新 | 从 `IndexChartDialog.tsx` 抽出的图, 加虚线段/基准标记/tooltip 附加行 |
| `doudou_blend/src/ChartDialog.tsx` | 新 | 从 `IndexChartDialog.tsx` 抽出的弹层外壳 + 时间范围按钮 |
| `doudou_blend/src/IndexChartDialog.tsx` | 改 | 改用上面两个组件, 外观不变 |
| `doudou_blend/src/CoalPriceDialog.tsx` (+test) | 新 | 煤价走势弹层 |
| `doudou_blend/src/CoalPriceLine.tsx` (+test) | 改 | 可选 `onOpen`, 变按钮 |
| `doudou_blend/src/screens/CoalPoolScreen.tsx` (+test) | 改 | 打开/关闭煤价弹层 |
| `doudou_blend/src/App.css` | 改 | 估价行按钮、图注样式 |
| `CLAUDE.md` | 改 | 架构段 `coal_prices.rs` 那句 |

---

### Task 1: `benchmarks.rs` —— 按日期取基准

**Files:** Modify `blend_kit_server/src/benchmarks.rs` (`latest_stored` 约 198 行、`current_from_pool` 约 240 行、tests 模块)

- [ ] **Step 1: 写失败测试** (加在 `benchmarks.rs` 的 `mod tests` 里, 复用已有的 `point` / `index_config` / `index_history`)

```rust
    fn inputs_fixture() -> (Value, BenchmarkInputs) {
        let master = json!({"price_factor": {
            "adjusts": [],
            "index_series": index_config(),
            "benchmarks": [
                {"name": "晋中", "coal_type": "焦煤", "props": {"S": 0.6}, "date": "2026-10-16", "price": 2300.0}
            ]
        }});
        let inputs = BenchmarkInputs {
            stored: vec![json!({"name": "孝义高硫", "coal_type": "焦煤", "props": {"S": 2.9},
                                "date": "2026-10-16", "price": 2039.0, "origin": "stored"})],
            history: HashMap::from([
                ("LJM_DLZJM".to_string(), vec![point("2026-10-15", 2294.0), point("2026-10-20", 2300.0)]),
                ("LJM_GLZJM".to_string(), vec![point("2026-10-15", 2118.6), point("2026-10-20", 2110.0)]),
            ]),
            ccp: Some(vec![point("2026-10-08", 1907.0), point("2026-10-15", 1900.0)]),
        };
        (master, inputs)
    }

    fn find<'a>(benchmarks: &'a [Value], name: &str) -> &'a Value {
        benchmarks.iter().find(|b| b["name"] == name).unwrap()
    }

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn test_as_of_ignores_everything_after_the_day() {
        let (master, inputs) = inputs_fixture();
        let before = as_of(&master, &inputs, day("2026-10-20"));
        let mut future = BenchmarkInputs {
            stored: inputs.stored.clone(),
            history: inputs.history.clone(),
            ccp: inputs.ccp.clone(),
        };
        // 晚于 10-20 的: 新报价、同名的新一期报价、指数新一期、CCP 新一期.
        future.stored.push(json!({"name": "未来报价", "coal_type": "焦煤", "props": {"S": 1.0},
                                  "date": "2026-10-25", "price": 9999.0, "origin": "stored"}));
        future.stored.push(json!({"name": "孝义高硫", "coal_type": "焦煤", "props": {"S": 2.9},
                                  "date": "2026-10-26", "price": 1.0, "origin": "stored"}));
        future.history.get_mut("LJM_GLZJM").unwrap().push(point("2026-10-26", 1.0));
        future.history.get_mut("LJM_DLZJM").unwrap().push(point("2026-10-26", 1.0));
        future.ccp.as_mut().unwrap().push(point("2026-10-23", 1.0));
        let mut master_future = master.clone();
        master_future["price_factor"]["benchmarks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "master 未来", "props": {}, "date": "2026-10-30", "price": 1.0}));
        assert_eq!(as_of(&master_future, &future, day("2026-10-20")), before);
    }

    #[test]
    fn test_as_of_truncates_index_and_drift_at_the_day() {
        let (master, inputs) = inputs_fixture();
        let on_17 = as_of(&master, &inputs, day("2026-10-17"));
        // 10-17 时指数最新一期是 10-15: 指数基准取 10-15 的值.
        assert_eq!(find(&on_17, "高硫主焦煤指数")["price"], 2118.6);
        assert_eq!(find(&on_17, "高硫主焦煤指数")["date"], "2026-10-15");
        // 10-16 的报价, 10-17 时还没有更新一期的指数: 不推算.
        assert!(find(&on_17, "孝义高硫").get("current_price").is_none());
        let on_20 = as_of(&master, &inputs, day("2026-10-20"));
        // 10-20 时: 高硫档 10-15 → 10-20 推算, 比例只到 10-20.
        assert_eq!(find(&on_20, "孝义高硫")["index_date"], "2026-10-20");
        assert_eq!(find(&on_20, "孝义高硫")["current_price"], json!((2039.0 * (2110.0 / 2118.6) * 100.0_f64).round() / 100.0));
        // 报价日晚于这天的不出现.
        assert!(as_of(&master, &inputs, day("2026-10-15")).iter().all(|b| b["name"] != "孝义高硫" && b["name"] != "晋中"));
    }

    #[test]
    fn test_as_of_today_matches_untruncated_pipeline() {
        let (master, inputs) = inputs_fixture();
        let config = master["price_factor"]["index_series"].as_array().unwrap().clone();
        let indexed = index_benchmarks(&config, &inputs.history);
        let mut expected = merge(&[&master_benchmarks(&master), &inputs.stored, &indexed]);
        apply_drift(&mut expected, |b| {
            drift_series(b, &config, &inputs.history, inputs.ccp.as_deref())
        });
        assert_eq!(as_of(&master, &inputs, day("2026-10-30")), expected);
    }

    #[test]
    fn test_same_day_stored_quotes_latest_created_wins() {
        // all_stored 按 name, quoted_on, created_at 升序给出; merge 同日后来者胜.
        let stored = [
            json!({"name": "晋中", "date": "2026-10-10", "price": 2000, "id": "先录"}),
            json!({"name": "晋中", "date": "2026-10-10", "price": 2010, "id": "后录"}),
        ];
        let merged = merge(&[&stored]);
        assert_eq!(merged[0]["id"], "后录");
    }
```

- [ ] **Step 2: 跑测试确认 RED**

Run: `cd blend_kit_server && cargo test --locked as_of`
Expected: 编译失败, `BenchmarkInputs` / `as_of` 未定义。

- [ ] **Step 3: 实现**

把 `latest_stored` 换成读全部报价 (排序是同日 `created_at` 胜出的关键, 注释写明):

```rust
/// 库表全部报价 (按日回放要看到每一条, 不止最新). 升序排: merge 同名同日时后来者胜,
/// 所以同日仍是 created_at 最新的那条生效.
async fn all_stored(pool: &PgPool) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, coal_type, form, region, props, price, quoted_on, source, created_by
        FROM price_benchmarks
        ORDER BY name, quoted_on, created_at
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(stored_row).collect()
}
```

新增输入结构、读取函数与纯函数, `current_from_pool` 改为薄包装:

```rust
/// 定基准要的全部原料, 一次读齐; 按日回放时每天复用.
pub(crate) struct BenchmarkInputs {
    pub stored: Vec<Value>,
    pub history: HashMap<String, Vec<IndexPoint>>,
    pub ccp: Option<Vec<IndexPoint>>,
}

/// 读库失败只记日志、退回 master 那一层, 错误另行交给后台任务 (它们不能拿降级的结果写库).
pub(crate) async fn load_inputs(pool: Option<&PgPool>) -> (BenchmarkInputs, Option<String>) {
    let mut read_error = None;
    let (stored, history) = match pool {
        Some(pool) => {
            let stored = all_stored(pool).await.unwrap_or_else(|error| {
                tracing::error!(%error, "读取基准报价失败, 只用 master 默认值");
                read_error = Some(error.to_string());
                Vec::new()
            });
            let history = mcci::history(pool).await.unwrap_or_else(|error| {
                tracing::error!(%error, "读取钢铁网指数失败, 不用指数基准");
                read_error = Some(error.to_string());
                HashMap::new()
            });
            (stored, history)
        }
        None => (Vec::new(), HashMap::new()),
    };
    let ccp = coal_index::get_spot_series().await;
    (BenchmarkInputs { stored, history, ccp }, read_error)
}

fn known_by(day: &str) -> impl Fn(&Value) -> bool + '_ {
    move |benchmark| benchmark["date"].as_str().is_some_and(|date| date <= day)
}

fn points_until(points: &[IndexPoint], day: &str) -> Vec<IndexPoint> {
    points.iter().filter(|point| point.date.as_str() <= day).cloned().collect()
}

/// 某天 (北京时间) 能拿到的基准: 报价日、指数期都不晚于这天, 旧报价也只推算到这天.
/// 实时快照就是 as_of(今天); 补算历史逐日调用, 两边一套口径.
pub(crate) fn as_of(master: &Value, inputs: &BenchmarkInputs, day: NaiveDate) -> Vec<Value> {
    let day = day.format("%Y-%m-%d").to_string();
    let known = known_by(&day);
    let master_layer: Vec<Value> = master_benchmarks(master).into_iter().filter(|b| known(b)).collect();
    let stored: Vec<Value> = inputs.stored.iter().filter(|b| known(b)).cloned().collect();
    let history: HashMap<String, Vec<IndexPoint>> = inputs
        .history
        .iter()
        .map(|(code, points)| (code.clone(), points_until(points, &day)))
        .filter(|(_, points)| !points.is_empty())
        .collect();
    let ccp = inputs
        .ccp
        .as_deref()
        .map(|points| points_until(points, &day))
        .filter(|points| !points.is_empty());
    let config = master["price_factor"]["index_series"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let indexed = index_benchmarks(&config, &history);
    let mut merged = merge(&[&master_layer, &stored, &indexed]);
    apply_drift(&mut merged, |benchmark| {
        drift_series(benchmark, &config, &history, ccp.as_deref())
    });
    merged
}

/// 当前生效的基准 = as_of(北京时间今天). 读库失败的降级见 [`load_inputs`].
pub(crate) async fn current_from_pool(
    pool: Option<&PgPool>,
    master: &Value,
) -> (Vec<Value>, Option<String>) {
    let (inputs, read_error) = load_inputs(pool).await;
    let today = Utc::now().with_timezone(&mcci::beijing()).date_naive();
    (as_of(master, &inputs, today), read_error)
}
```

`use chrono::NaiveDate;` 改为 `use chrono::{NaiveDate, Utc};`。删掉不再使用的 `latest_stored`。更新 `current_from_pool` 上方原有的文档注释 (三层来源的说明保留, 移到 `as_of` 上)。

- [ ] **Step 4: 跑测试确认 GREEN, 且原有基准测试不回归**

Run: `cd blend_kit_server && cargo test --locked benchmarks`
Expected: 新增 4 项 + 原有全部通过。

---

### Task 2: 迁移与补算的纯逻辑

**Files:** Create `blend_kit_server/migrations/0012_coal_price_source.sql`; Modify `blend_kit_server/src/mcci.rs:60` (`HISTORY_DAYS` 改 `pub(crate) const`), `blend_kit_server/src/coal_prices.rs`

- [ ] **Step 1: 写迁移**

```sql
-- 估价快照的来历: live = 当天每小时重算、过零点定格的实时快照; backfill = 事后按"当天能拿到的
-- 指数与报价"补算的 (服务停机的日子、新加的煤、上线前的一年). 两种都存下就不改.
ALTER TABLE coal_price_daily
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'live'
    CHECK (source IN ('live', 'backfill'));
```

`write_snapshot` 不用改: 它不写 `source`, 取默认 `live`。在它的文档注释里补一句"实时行, source 取默认 live"。

- [ ] **Step 2: 写失败测试** (加在 `coal_prices.rs` 的 `mod tests`, 复用已有 `fixture()`)

```rust
    fn d(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn test_backfill_range_caps_at_history_window_and_stops_before_today() {
        let today = d("2026-10-30");
        assert_eq!(backfill_range(today, Some(d("2025-10-27"))), Some((d("2025-10-27"), d("2026-10-29"))));
        // 输入比 400 天还早: 起点截到 400 天前.
        assert_eq!(backfill_range(today, Some(d("2020-01-01"))), Some((d("2025-09-25"), d("2026-10-29"))));
        // 没有输入, 或最早的输入就是今天: 不补.
        assert_eq!(backfill_range(today, None), None);
        assert_eq!(backfill_range(today, Some(today)), None);
    }

    #[test]
    fn test_earliest_input_takes_min_over_index_and_quotes() {
        let master = json!({"price_factor": {"benchmarks": [{"name": "m", "date": "2026-10-09"}]}});
        let inputs = benchmarks::BenchmarkInputs {
            stored: vec![json!({"name": "s", "date": "2026-10-20"})],
            history: HashMap::from([(
                "LJM_QM".to_string(),
                vec![IndexPoint { date: "2025-10-27".into(), price: 1.0 }],
            )]),
            ccp: Some(vec![IndexPoint { date: "2024-01-01".into(), price: 1.0 }]),
        };
        // CCP 单独定不出价, 不算输入.
        assert_eq!(earliest_input(&master, &inputs), Some(d("2025-10-27")));
        let empty = benchmarks::BenchmarkInputs { stored: vec![], history: HashMap::new(), ccp: None };
        assert_eq!(earliest_input(&json!({}), &empty), None);
    }

    #[test]
    fn test_missing_by_day_only_lists_gaps() {
        let names = vec!["甲".to_string(), "乙".to_string()];
        let existing = HashSet::from([
            (d("2026-10-01"), "甲".to_string()),
            (d("2026-10-01"), "乙".to_string()),
            (d("2026-10-02"), "甲".to_string()),
        ]);
        let plan = missing_by_day((d("2026-10-01"), d("2026-10-03")), &names, &existing);
        assert_eq!(
            plan,
            vec![
                (d("2026-10-02"), BTreeSet::from(["乙".to_string()])),
                (d("2026-10-03"), BTreeSet::from(["甲".to_string(), "乙".to_string()])),
            ]
        );
    }

    #[test]
    fn test_backfill_rows_price_each_day_as_of_that_day_and_skip_failures() {
        let (mut master, _) = fixture();
        master["price_factor"]["index_series"] = json!([]);
        master["price_factor"]["benchmarks"] = json!([
            {"name": "旧基准", "coal_type": "主焦煤", "props": {"S": 1.0, "Y": 20.0}, "price": 1000.0, "date": "2026-10-02"}
        ]);
        let inputs = benchmarks::BenchmarkInputs {
            stored: vec![json!({"name": "新基准", "coal_type": "主焦煤", "props": {"S": 1.0, "Y": 20.0},
                                "price": 2000.0, "date": "2026-10-03", "origin": "stored"})],
            history: HashMap::new(),
            ccp: None,
        };
        let plan = vec![
            (d("2026-10-01"), BTreeSet::from(["主力".to_string()])),          // 一个基准都没有: 跳过
            (d("2026-10-02"), BTreeSet::from(["主力".to_string()])),          // 只有旧基准
            (d("2026-10-03"), BTreeSet::from(["主力".to_string(), "停用".to_string()])),
        ];
        let (rows, skipped) = backfill_rows(&master, &inputs, &plan);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, d("2026-10-01"));
        let got: Vec<_> = rows.iter().map(|(day, row)| (day.to_string(), row.name.clone(), row.price)).collect();
        assert_eq!(
            got,
            vec![
                ("2026-10-02".into(), "主力".into(), 1000.0),
                ("2026-10-03".into(), "主力".into(), 2000.0),
                ("2026-10-03".into(), "停用".into(), 2020.0),
            ]
        );
    }
```

(数字的来历: fixture 系数下 Y 21 比标准 20 多 10 点, 与 `test_snapshot_rows_include_archived_and_missing_indicators` 里
「停用」2100 → 2121 的比例 1.01 相同, 所以 2000 → 2020。10-03 两条基准指标相同、距离并列, `merge` 按 (煤种, 名字) 排序后
「新基准」在「旧基准」前 (按字节 新 < 旧), 并列取先列的, 所以用 2000。)

- [ ] **Step 3: 跑测试确认 RED**

Run: `cd blend_kit_server && cargo test --locked backfill`
Expected: 编译失败, `backfill_range` 等未定义。

- [ ] **Step 4: 实现纯函数** (`coal_prices.rs`; `use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};`、`use chrono::Days;`、`use crate::coal_index::IndexPoint;` 仅测试用的放进 tests 模块)

```rust
/// 补算范围: 最早一条能定价的输入 (指数各序列首日、master 与库表报价日) 与 400 天前取晚的,
/// 到昨天为止 —— 今天归实时快照管. 没有输入、或起点不早于今天: 不补.
fn backfill_range(today: NaiveDate, earliest: Option<NaiveDate>) -> Option<(NaiveDate, NaiveDate)> {
    let start = earliest?.max(today - Days::new(mcci::HISTORY_DAYS));
    let end = today.pred_opt()?;
    (start <= end).then_some((start, end))
}

/// CCP 只用来推算涨跌, 单独定不出价, 不算输入.
fn earliest_input(master: &Value, inputs: &benchmarks::BenchmarkInputs) -> Option<NaiveDate> {
    let quotes = master["price_factor"]["benchmarks"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(&inputs.stored)
        .filter_map(|benchmark| benchmark["date"].as_str());
    let index = inputs.history.values().filter_map(|points| points.first()).map(|point| point.date.as_str());
    quotes
        .chain(index)
        .filter_map(|date| date.parse::<NaiveDate>().ok())
        .min()
}

/// 范围内逐日列出还缺的煤; 一种都不缺的天不出现.
fn missing_by_day(
    (start, end): (NaiveDate, NaiveDate),
    names: &[String],
    existing: &HashSet<(NaiveDate, String)>,
) -> Vec<(NaiveDate, BTreeSet<String>)> {
    start
        .iter_days()
        .take_while(|day| *day <= end)
        .filter_map(|day| {
            let missing: BTreeSet<String> = names
                .iter()
                .filter(|name| !existing.contains(&(day, (*name).clone())))
                .cloned()
                .collect();
            (!missing.is_empty()).then_some((day, missing))
        })
        .collect()
}

/// 逐日按 as_of(那天) 估价, 只留缺的煤; 估不出价的天 (如一个基准都没有) 跳过并记下原因.
fn backfill_rows(
    master: &Value,
    inputs: &benchmarks::BenchmarkInputs,
    plan: &[(NaiveDate, BTreeSet<String>)],
) -> (Vec<(NaiveDate, PriceRow)>, Vec<(NaiveDate, String)>) {
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for (day, missing) in plan {
        let benchmarks = benchmarks::as_of(master, inputs, *day);
        // 不能把空基准交给 snapshot_rows: fill_price_request 在基准为空时退回 master 自带的,
        // 那里有晚于这天的报价 —— 等于偷看未来.
        if benchmarks.is_empty() {
            skipped.push((*day, "这天还没有任何基准".into()));
            continue;
        }
        match snapshot_rows(master, &benchmarks) {
            Ok(priced) => rows.extend(
                priced
                    .into_iter()
                    .filter(|row| missing.contains(&row.name))
                    .map(|row| (*day, row)),
            ),
            Err(reason) => skipped.push((*day, reason)),
        }
    }
    (rows, skipped)
}
```

注意 `benchmarks::fill_price_request` 只在传进去的基准**非空**时才替换 master 自带的 (`&& !benchmarks.is_empty()`);
空的话 master 的 `price_factor.benchmarks` 原样进估价 —— 测试里 10-01 那天就会偷用 master 里 10-02 的「旧基准」估出 1000。
上面 `benchmarks.is_empty()` 的跳过就是为此; 测试的 10-01 一项专门钉住它, 不要删。`fill_price_request` 本身不改
(实时与 `/api/price` 依赖它的现有行为)。

- [ ] **Step 5: 跑测试确认 GREEN**

Run: `cd blend_kit_server && cargo test --locked coal_prices`
Expected: 新增 4 项 + 原有全部通过。

---

### Task 3: 补算的读写与调度

**Files:** Modify `blend_kit_server/src/coal_prices.rs` (`refresh` 之后、`spawn_hourly`)

这一步全是 IO, 单测覆盖不到, 由 Task 9 的真实 PostgreSQL 联调验证。

- [ ] **Step 1: 实现读写**

```rust
pub(crate) struct BackfillReport {
    pub days: usize,
    pub rows: u64,
    pub skipped: Vec<(NaiveDate, String)>,
}

async fn existing_keys(
    pool: &PgPool,
    (start, end): (NaiveDate, NaiveDate),
) -> Result<HashSet<(NaiveDate, String)>, sqlx::Error> {
    let rows: Vec<(NaiveDate, String)> =
        sqlx::query_as("SELECT day, name FROM coal_price_daily WHERE day BETWEEN $1 AND $2")
            .bind(start)
            .bind(end)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// 补算的行只填空: 已有的 (实时定格的、之前补过的) 一律不动. 每批自身幂等, 中途失败下次接着补.
async fn insert_backfill(pool: &PgPool, rows: &[(NaiveDate, PriceRow)]) -> Result<u64, sqlx::Error> {
    let mut inserted = 0;
    for chunk in rows.chunks(5000) {
        let days: Vec<_> = chunk.iter().map(|(day, _)| *day).collect();
        let names: Vec<_> = chunk.iter().map(|(_, row)| row.name.as_str()).collect();
        let prices: Vec<_> = chunk.iter().map(|(_, row)| row.price).collect();
        let points: Vec<_> = chunk.iter().map(|(_, row)| row.points).collect();
        let benchmarks: Vec<_> = chunk.iter().map(|(_, row)| row.benchmark.as_deref()).collect();
        let benchmark_prices: Vec<_> = chunk.iter().map(|(_, row)| row.benchmark_price).collect();
        let missing: Vec<_> = chunk.iter().map(|(_, row)| json!(row.missing)).collect();
        inserted += sqlx::query(
            r#"
            INSERT INTO coal_price_daily
                (day, name, price, points, benchmark, benchmark_price, missing, source)
            SELECT day, name, price, points, benchmark, benchmark_price, missing, 'backfill'
            FROM UNNEST($1::date[], $2::text[], $3::float8[], $4::float8[], $5::text[],
                        $6::float8[], $7::jsonb[])
                AS t(day, name, price, points, benchmark, benchmark_price, missing)
            ON CONFLICT (day, name) DO NOTHING
            "#,
        )
        .bind(&days)
        .bind(&names)
        .bind(&prices)
        .bind(&points)
        .bind(&benchmarks)
        .bind(&benchmark_prices)
        .bind(&missing)
        .execute(pool)
        .await?
        .rows_affected();
    }
    Ok(inserted)
}

/// 补齐 today 以前缺的 (日期, 煤). 输入读不全 (库表/指数读失败、CCP 取不到) 整轮不写:
/// 补下去就定格了, 不能拿缺料的结果填.
pub(crate) async fn backfill(pool: &PgPool, today: NaiveDate) -> Result<BackfillReport, String> {
    let master: Value = serde_json::from_str(&merged_master_json_from_pool(Some(pool)).await?)
        .map_err(|error| error.to_string())?;
    let (inputs, error) = benchmarks::load_inputs(Some(pool)).await;
    if let Some(error) = error {
        return Err(error);
    }
    if inputs.ccp.is_none() {
        return Err("CCP 指数取不到, 推算缺一层, 不补算".into());
    }
    let Some(range) = backfill_range(today, earliest_input(&master, &inputs)) else {
        return Ok(BackfillReport { days: 0, rows: 0, skipped: Vec::new() });
    };
    let names: Vec<String> = master["coals"]
        .as_array()
        .ok_or("煤库缺少煤种列表")?
        .iter()
        .filter_map(|coal| coal["name"].as_str().map(str::to_string))
        .collect();
    let existing = existing_keys(pool, range).await.map_err(|error| error.to_string())?;
    let plan = missing_by_day(range, &names, &existing);
    let days = plan.len();
    // 逐日估价是同步的 CPU 活, 不占异步线程.
    let (rows, skipped) =
        tokio::task::spawn_blocking(move || backfill_rows(&master, &inputs, &plan))
            .await
            .map_err(|error| error.to_string())?;
    let rows = insert_backfill(pool, &rows).await.map_err(|error| error.to_string())?;
    Ok(BackfillReport { days, rows, skipped })
}
```

(`BenchmarkInputs` 要能 `move` 进 `spawn_blocking`: 它只含 `Vec`/`HashMap`/`Option`, 天然 `Send + 'static`。)

- [ ] **Step 2: 调度并进 `spawn_hourly`**

```rust
/// interval 首次立即触发; 不受 MCCI_FETCH 开关影响, 每轮 panic 由子任务隔离.
/// 每轮先算今天 (卡片价优先), 再看今天补过历史没有: 启动时一次, 之后每过零点一次; 补失败下小时重试.
pub(crate) fn spawn_hourly(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        let mut backfilled_on: Option<NaiveDate> = None;
        loop {
            ticker.tick().await;
            let round_pool = pool.clone();
            match tokio::spawn(async move { refresh(&round_pool).await }).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!(%error, "每日估价快照更新失败, 保留旧数据"),
                Err(error) => tracing::error!(%error, "每日估价快照这一轮异常退出, 下个小时再来"),
            }
            let today = beijing_day(Utc::now());
            if backfilled_on == Some(today) {
                continue;
            }
            let round_pool = pool.clone();
            match tokio::spawn(async move { backfill(&round_pool, today).await }).await {
                Ok(Ok(report)) => {
                    let first_skipped = report.skipped.first().map(|(day, reason)| format!("{day}: {reason}"));
                    tracing::info!(
                        days = report.days,
                        rows = report.rows,
                        skipped = report.skipped.len(),
                        first_skipped = first_skipped.as_deref().unwrap_or("-"),
                        "历史估价补算完成"
                    );
                    backfilled_on = Some(today);
                }
                Ok(Err(error)) => tracing::error!(%error, "历史估价补算失败, 下个小时再试"),
                Err(error) => tracing::error!(%error, "历史估价补算这一轮异常退出, 下个小时再试"),
            }
        }
    });
}
```

- [ ] **Step 3: 编译与已有测试**

Run: `cd blend_kit_server && cargo test --locked coal_prices && cargo clippy --release -- -D warnings`
Expected: 通过, 无警告。

---

### Task 4: 历史接口 `GET /api/coal-prices/history`

**Files:** Modify `blend_kit_server/src/coal_prices.rs`, `blend_kit_server/src/lib.rs:248` (路由) 与其 tests (`test_coal_prices_requires_login_and_database` 旁), `scripts/check_repository_consistency.mjs:128`

- [ ] **Step 1: 写失败测试**

`lib.rs` tests 里, 紧跟 `test_coal_prices_requires_login_and_database`:

```rust
    #[tokio::test]
    async fn test_coal_price_history_checks_login_then_name_then_database() {
        for (token, uri, expected) in [
            ("forged", "/api/coal-prices/history?name=%E4%B8%B4%E5%8C%97", StatusCode::UNAUTHORIZED),
            ("test-session-token", "/api/coal-prices/history", StatusCode::BAD_REQUEST),
            ("test-session-token", "/api/coal-prices/history?name=", StatusCode::BAD_REQUEST),
            ("test-session-token", "/api/coal-prices/history?name=%E4%B8%B4%E5%8C%97", StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = test_app()
                .oneshot(session_request("GET", uri, token, Body::empty()))
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{uri}");
            let body: Value = serde_json::from_str(&response_text(response).await).unwrap();
            assert_eq!(body["ok"], false);
        }
    }
```

`coal_prices.rs` tests 里 (响应形状):

```rust
    #[test]
    fn test_history_response_shape() {
        let body = json!(CoalPriceHistory {
            ok: true,
            name: "临北".into(),
            points: vec![
                CoalPricePoint { day: d("2025-10-27"), price: 2050.3, benchmark: Some("低硫主焦煤指数".into()), source: "backfill".into() },
                CoalPricePoint { day: d("2026-10-30"), price: 2214.0, benchmark: None, source: "live".into() },
            ],
        });
        assert_eq!(body["points"][0], json!({"day": "2025-10-27", "price": 2050.3, "benchmark": "低硫主焦煤指数", "source": "backfill"}));
        assert!(body["points"][1]["benchmark"].is_null());
    }
```

- [ ] **Step 2: 跑测试确认 RED**

Run: `cd blend_kit_server && cargo test --locked coal_price_history history_response`
Expected: 编译失败 / 404。

- [ ] **Step 3: 实现** (`coal_prices.rs`; `use axum::extract::Query;`、`use serde::Deserialize;`、`use super::bad_request;`)

```rust
/// /api/coal-prices/history 的一天. source: live = 实时快照定格, backfill = 事后补算.
#[derive(Serialize, sqlx::FromRow)]
pub struct CoalPricePoint {
    pub day: NaiveDate,
    pub price: f64,
    pub benchmark: Option<String>,
    pub source: String,
}

/// 一种煤的历史估价, 按日升序; 煤库里没有这种煤时为空.
#[derive(Serialize)]
pub struct CoalPriceHistory {
    pub ok: bool,
    pub name: String,
    pub points: Vec<CoalPricePoint>,
}

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    name: Option<String>,
}

async fn read_history(pool: &PgPool, name: &str) -> Result<Vec<CoalPricePoint>, sqlx::Error> {
    sqlx::query_as(
        "SELECT day, price, benchmark, source FROM coal_price_daily WHERE name = $1 ORDER BY day",
    )
    .bind(name)
    .fetch_all(pool)
    .await
}

pub(crate) async fn history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Response {
    if !is_authenticated(&headers, &state.auth) {
        return unauthorized();
    }
    let Some(name) = query.name.filter(|name| !name.is_empty()) else {
        return bad_request("缺少煤名 name");
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    match read_history(pool, &name).await {
        Ok(points) => Json(json!(CoalPriceHistory { ok: true, name, points })).into_response(),
        Err(error) => database_error(error),
    }
}
```

`lib.rs` 路由 (紧跟 `/api/coal-prices`):

```rust
        .route("/api/coal-prices/history", get(coal_prices::history))
```

`scripts/check_repository_consistency.mjs` 的 `sharedStructs` 开头加:

```js
  [rustCoalPricesSource, "CoalPricePoint", "CoalPricePoint"],
  [rustCoalPricesSource, "CoalPriceHistory", "CoalPriceHistory"],
```

- [ ] **Step 4: 跑测试确认 GREEN**

Run: `cd blend_kit_server && cargo test --locked coal_price`
Expected: 全部通过。(`check:consistency` 要等 Task 5 加了 TS 类型才过。)

---

### Task 5: 前端数据层

**Files:** Modify `doudou_blend/src/types.ts` (紧跟 `CoalPriceSnapshot`), `doudou_blend/src/coalPrices.ts`, `doudou_blend/src/coalPrices.test.ts`, `doudou_blend/src/mcci.ts:60`

- [ ] **Step 1: 类型**

```ts
/** /api/coal-prices/history 的一天: live = 当天实时快照定格, backfill = 事后按当天能拿到的指数与报价补算. */
export interface CoalPricePoint {
  day: string;
  price: number;
  benchmark: string | null;
  source: "live" | "backfill";
}

/** 一种煤的历史估价, 按日升序; 煤库里没有这种煤时为空. */
export interface CoalPriceHistory {
  ok: boolean;
  name: string;
  points: CoalPricePoint[];
}
```

- [ ] **Step 2: 写失败测试** (`coalPrices.test.ts`, 照文件里 `fetchCoalPrices` 测试的 `vi.stubGlobal("fetch", ...)` 写法)

```ts
describe("单煤历史估价", () => {
  it("煤名做 URL 编码, 返回升序的点", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      ok: true, name: "1/3焦 临北", points: [{ day: "2026-10-29", price: 2214, benchmark: "安泽低硫主焦", source: "live" }],
    })));
    vi.stubGlobal("fetch", fetchMock);
    const history = await fetchCoalPriceHistory("1/3焦 临北");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/coal-prices/history?name=1%2F3%E7%84%A6%20%E4%B8%B4%E5%8C%97");
    expect(history?.points).toHaveLength(1);
  });

  it.each([
    ["服务异常", () => Promise.resolve(new Response("{}", { status: 503 }))],
    ["形状不对", () => Promise.resolve(new Response(JSON.stringify({ ok: true })))],
    ["离线", () => Promise.reject(new TypeError("offline"))],
  ])("%s 时返回 null", async (_, impl) => {
    vi.stubGlobal("fetch", vi.fn().mockImplementation(impl));
    expect(await fetchCoalPriceHistory("临北")).toBeNull();
  });

  it("只有共享煤库里有快照的煤才有历史; 用户自己新增的没有", () => {
    expect(hasPriceHistory(coal, snapshot)).toBe(true);
    expect(hasPriceHistory(coal, null)).toBe(false);
    expect(hasPriceHistory(resolveCoal(base, null, "user"), snapshot)).toBe(false);
    expect(hasPriceHistory(resolveCoal({ ...base, name: "别的" }, null, "master"), snapshot)).toBe(false);
  });
});
```

(若 `resolveCoal` 的第三个参数取值不是 `"user"` / `"master"`, 以 `domain/resolvedCoal.ts` 为准改测试的构造方式, 断言不变。)

- [ ] **Step 3: 跑测试确认 RED**

Run: `cd doudou_blend && npx vitest run src/coalPrices.test.ts`
Expected: FAIL, `fetchCoalPriceHistory` / `hasPriceHistory` 未导出。

- [ ] **Step 4: 实现** (`coalPrices.ts`)

```ts
export async function fetchCoalPriceHistory(name: string): Promise<CoalPriceHistory | null> {
  try {
    const response = await fetch(`/api/coal-prices/history?name=${encodeURIComponent(name)}`, {
      credentials: "same-origin", cache: "no-store",
    });
    if (!response.ok) return null;
    const result = (await response.json()) as CoalPriceHistory;
    return result.ok && Array.isArray(result.points) ? result : null;
  } catch {
    return null;
  }
}

/** 历史按共享煤库记; 用户自己新增的煤 (哪怕同名) 没有自己的历史. */
export function hasPriceHistory(coal: ResolvedCoal, snapshot: CoalPriceSnapshot | null): boolean {
  return coal.origin !== "user" && !!snapshot?.prices.some((entry) => entry.name === coal.name);
}
```

`isModified` 前加 `export` (煤价弹层要用)。`mcci.ts` 的 `lastDays` 改泛型, 实现不变:

```ts
export function lastDays<T extends McciPoint>(points: T[], days: number): T[] {
```

- [ ] **Step 5: GREEN + 一致性**

Run: `cd doudou_blend && npx vitest run src/coalPrices.test.ts src/mcci.test.ts && npm run check:consistency`
Expected: 通过 (若无 `mcci.test.ts` 则略去)。

---

### Task 6: 抽出共用的图与弹层外壳

**Files:** Create `doudou_blend/src/PriceChart.tsx`, `doudou_blend/src/PriceChart.test.tsx`, `doudou_blend/src/ChartDialog.tsx`; Modify `doudou_blend/src/IndexChartDialog.tsx`

- [ ] **Step 1: 写失败测试** (`PriceChart.test.tsx`)

```tsx
// @vitest-environment jsdom

import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { PriceChart, type ChartPoint } from "./PriceChart";

afterEach(cleanup);

const plain: ChartPoint[] = [
  { date: "2026-10-27", price: 2200 }, { date: "2026-10-28", price: 2210 }, { date: "2026-10-29", price: 2214 },
];

describe("走势图", () => {
  it("不传新参数时只有一条实线, 没有基准标记", () => {
    const { container } = render(<PriceChart label="测试" points={plain} />);
    const lines = container.querySelectorAll(".index-chart-line");
    expect(lines).toHaveLength(1);
    expect(lines[0].getAttribute("stroke-dasharray")).toBeNull();
    expect(container.querySelectorAll(".price-chart-marker")).toHaveLength(0);
  });

  it("回算段画虚线, 与实线段共用交界点", () => {
    const points: ChartPoint[] = [
      { date: "2026-10-27", price: 2200, dashed: true },
      { date: "2026-10-28", price: 2210, dashed: true },
      { date: "2026-10-29", price: 2214 },
      { date: "2026-10-30", price: 2220 },
    ];
    const { container } = render(<PriceChart label="测试" points={points} />);
    const lines = [...container.querySelectorAll(".index-chart-line")];
    expect(lines.map((line) => line.getAttribute("stroke-dasharray"))).toEqual(["4 3", null]);
    const coords = lines.map((line) => line.getAttribute("points")!.split(" "));
    expect(coords[0]).toHaveLength(3);
    expect(coords[1]).toHaveLength(2);
    expect(coords[0][2]).toBe(coords[1][0]);
  });

  it("基准变更处画竖标; tooltip 显示附加行", () => {
    const points: ChartPoint[] = plain.map((point, i) => ({ ...point, notes: [i < 2 ? "对标 甲" : "对标 乙", "回算"] }));
    const { container, getByRole } = render(<PriceChart label="测试" points={points} markers={[2]} />);
    expect(container.querySelectorAll(".price-chart-marker")).toHaveLength(1);
    const svg = container.querySelector("svg")!;
    fireEvent.keyDown(svg, { key: "End" });
    expect(getByRole("status").textContent).toContain("对标 乙");
    expect(getByRole("status").textContent).toContain("回算");
  });
});
```

- [ ] **Step 2: 跑测试确认 RED**

Run: `cd doudou_blend && npx vitest run src/PriceChart.test.tsx`
Expected: FAIL, 模块不存在。

- [ ] **Step 3: 实现**

`PriceChart.tsx`: 把 `IndexChartDialog.tsx` 里的 `CHART_H` / `FALLBACK_W` / `PAD` / `Y_TICKS` / `X_LABELS` / `niceTicks` / `labelIndexes` / `useWidth` / `PriceChart` 原样搬过来 (文件头注释同步搬, 说明它现在被指数弹层与煤价弹层共用), `PriceChart` 导出并改为:

```tsx
export interface ChartPoint {
  date: string;
  price: number;
  /** 画成虚线 (如回算的估价); 不传为实线. */
  dashed?: boolean;
  /** tooltip 里价格、日期之后多显示的几行. */
  notes?: string[];
}

/** 相邻同为虚线 (或同为实线) 的点连成一段; 段与段共用交界点, 线不断开. */
function runs(points: ChartPoint[]): { dashed: boolean; from: number; to: number }[] {
  const result: { dashed: boolean; from: number; to: number }[] = [];
  let from = 0;
  for (let i = 1; i < points.length; i++) {
    if (!!points[i].dashed !== !!points[from].dashed) {
      result.push({ dashed: !!points[from].dashed, from, to: i });
      from = i;
    }
  }
  result.push({ dashed: !!points[from].dashed, from, to: points.length - 1 });
  return result;
}

export function PriceChart({
  label,
  points,
  markers = [],
}: {
  label: string;
  points: ChartPoint[];
  /** 要在横轴上方画竖标的点下标 (如对标基准变了的那天). */
  markers?: number[];
}) {
```

原来那一条 `<polyline className="index-chart-line" .../>` 换成:

```tsx
        {runs(points).map(({ dashed, from, to }) => (
          <polyline
            key={from}
            className="index-chart-line"
            points={points
              .slice(from, to + 1)
              .map((point, k) => `${x(from + k).toFixed(1)},${y(point.price).toFixed(1)}`)
              .join(" ")}
            fill="none"
            stroke="var(--c-primary)"
            strokeWidth={2}
            strokeDasharray={dashed ? "4 3" : undefined}
            strokeLinejoin="round"
            strokeLinecap="round"
          />
        ))}
        {markers.map((i) => (
          <line
            key={`marker-${i}`}
            className="price-chart-marker"
            x1={x(i)}
            x2={x(i)}
            y1={PAD.top + innerH - 6}
            y2={PAD.top + innerH}
            stroke="var(--c-text-3)"
            strokeWidth={1.5}
          />
        ))}
```

(全为实线时 `runs` 只有一段, 输出与原来完全相同, 原 `line` 变量删掉。) tooltip 里 `<span>{hovered.date}</span>` 之后加:

```tsx
          {hovered.notes?.map((note) => <span key={note}>{note}</span>)}
```

`ChartDialog.tsx`: 把 `IndexChartDialog` 的弹层外壳 (`onCloseRef`、焦点进出与 Esc 的 effect、`modal-backdrop` / `modal-sheet` / `modal-header` / 关闭按钮 / `modal-body index-chart-body`) 与 `RANGES` / `DEFAULT_RANGE` / 范围按钮组抽出来:

```tsx
/** 时间范围 (日历天). */
export const RANGES: [label: string, days: number][] = [["1月", 30], ["3月", 91], ["6月", 182], ["1年", 365]];
export const DEFAULT_RANGE = "1年";

export function rangeDays(range: string): number {
  return RANGES.find(([label]) => label === range)?.[1] ?? 365;
}

export function RangeButtons({ range, onChange }: { range: string; onChange: (range: string) => void }) {
  // 原 index-chart-ranges 那段 JSX, setRange 换成 onChange
}

export function ChartDialog({
  titleId, title, subtitle, onClose, children,
}: {
  titleId: string;
  title: string;
  subtitle: string;
  onClose: () => void;
  children: ReactNode;  // import type { ReactNode } from "react"
}) {
  // 原外壳 JSX: children 放进 <div className="modal-body index-chart-body">
}
```

`IndexChartDialog.tsx` 改为用 `ChartDialog` + `RangeButtons` + `PriceChart`, 渲染出的 DOM 与原来一致 (`titleId="index-chart-title"`)。

- [ ] **Step 4: GREEN, 且指数弹层测试原样通过**

Run: `cd doudou_blend && npx vitest run src/PriceChart.test.tsx src/IndexTicker.test.tsx`
Expected: 全部通过; `IndexTicker.test.tsx` 一行未改。

---

### Task 7: 煤价走势弹层 `CoalPriceDialog`

**Files:** Create `doudou_blend/src/CoalPriceDialog.tsx`, `doudou_blend/src/CoalPriceDialog.test.tsx`

- [ ] **Step 1: 写失败测试**

```tsx
// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CoalPriceDialog } from "./CoalPriceDialog";
import type { CoalPriceHistory } from "./types";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

function serve(body: unknown, status = 200) {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify(body), { status })));
}

const history: CoalPriceHistory = {
  ok: true, name: "临北", points: [
    { day: "2026-10-13", price: 2150, benchmark: "低硫主焦煤指数", source: "backfill" },
    { day: "2026-10-14", price: 2160, benchmark: "低硫主焦煤指数", source: "backfill" },
    { day: "2026-10-15", price: 2190, benchmark: "安泽低硫主焦", source: "backfill" },
    { day: "2026-10-29", price: 2214, benchmark: "安泽低硫主焦", source: "live" },
  ],
};

describe("煤价走势弹层", () => {
  it("显示最新价、对标基准、区间高低, 基准变更处有标记, 有回算说明", async () => {
    serve(history);
    const { container } = render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    expect(await screen.findByText("2,214")).toBeTruthy();
    expect(screen.getByText("对标 安泽低硫主焦")).toBeTruthy();
    expect(container.textContent).toContain("最高 2,214");
    expect(container.textContent).toContain("最低 2,150");
    expect(container.querySelectorAll(".price-chart-marker")).toHaveLength(1);
    expect(container.textContent).toContain("虚线为回算");
    expect(container.textContent).not.toContain("不含你的修改");
  });

  it("改过化验值的煤注明历史口径", async () => {
    serve(history);
    const { container } = render(<CoalPriceDialog name="临北" modified onClose={() => {}} />);
    await screen.findByText("2,214");
    expect(container.textContent).toContain("历史为共享煤库口径, 不含你的修改");
  });

  it.each<[string, () => void, string]>([
    ["取不到", () => serve({}, 503), "走势暂时取不到"],
    ["不足两天", () => serve({ ...history, points: history.points.slice(-1) }), "数据不足两天, 画不出走势"],
  ])("%s", async (_, setup, text) => {
    setup();
    render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    expect(await screen.findByText(text)).toBeTruthy();
  });

  it("请求回来前显示加载中", () => {
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise(() => {})));
    render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    expect(screen.getByText("加载中…")).toBeTruthy();
  });
});
```

- [ ] **Step 2: 跑测试确认 RED**

Run: `cd doudou_blend && npx vitest run src/CoalPriceDialog.test.tsx`
Expected: FAIL, 模块不存在。

- [ ] **Step 3: 实现**

```tsx
/**
 * 煤池卡片点开的单煤估价走势: 共享煤库口径的日度估价 (实时快照定格 + 事后补算),
 * 补算段画虚线, 对标基准换了的那天在横轴上方画竖标. 外壳与图与行情条的指数弹层共用.
 */
import { useEffect, useState } from "react";
import { ChartDialog, DEFAULT_RANGE, RangeButtons, rangeDays } from "./ChartDialog";
import { fetchCoalPriceHistory } from "./coalPrices";
import { lastDays } from "./mcci";
import { PriceChart, type ChartPoint } from "./PriceChart";
import type { CoalPriceHistory, CoalPricePoint } from "./types";

type Row = ChartPoint & { benchmark: string | null };

function toRow(point: CoalPricePoint): Row {
  const backfill = point.source === "backfill";
  return {
    date: point.day,
    price: point.price,
    benchmark: point.benchmark,
    dashed: backfill,
    notes: [point.benchmark ? `对标 ${point.benchmark}` : "按市场价估", ...(backfill ? ["回算"] : [])],
  };
}

const yuan = (price: number) => Math.round(price).toLocaleString("zh-CN");

export function CoalPriceDialog({ name, modified, onClose }: {
  name: string;
  /** 用户改过这种煤的化验值: 卡片价按修改值现算, 历史仍是共享煤库口径. */
  modified: boolean;
  onClose: () => void;
}) {
  const [history, setHistory] = useState<CoalPriceHistory | null | "loading">("loading");
  const [range, setRange] = useState(DEFAULT_RANGE);
  useEffect(() => {
    let alive = true;
    fetchCoalPriceHistory(name).then((result) => {
      if (alive) setHistory(result);
    });
    return () => {
      alive = false;
    };
  }, [name]);

  const all = history && history !== "loading" ? history.points.map(toRow) : [];
  const rows = lastDays(all, rangeDays(range));
  const latest = all[all.length - 1];
  const markers = rows.flatMap((row, i) => (i > 0 && row.benchmark !== rows[i - 1].benchmark ? [i] : []));
  const prices = rows.map((row) => row.price);

  const body =
    history === "loading" ? <p className="index-chart-empty">加载中…</p>
    : history === null ? <p className="index-chart-empty">走势暂时取不到</p>
    : rows.length < 2 ? <p className="index-chart-empty">数据不足两天, 画不出走势</p>
    : (
      <>
        <div className="index-chart-summary">
          <span className="index-chart-price">{yuan(latest.price)}</span>
          <span>元/吨 · {latest.date}</span>
          <span>最高 {yuan(Math.max(...prices))} · 最低 {yuan(Math.min(...prices))}</span>
        </div>
        <RangeButtons range={range} onChange={setRange} />
        <PriceChart key={range} label={`${name}估价`} points={rows} markers={markers} />
        {rows.some((row) => row.dashed) && (
          <p className="price-chart-note">虚线为回算: 按当天能拿到的指数和报价补算</p>
        )}
        {modified && <p className="price-chart-note">历史为共享煤库口径, 不含你的修改</p>}
      </>
    );

  return (
    <ChartDialog
      titleId="coal-price-title"
      title={name}
      subtitle={latest ? (latest.benchmark ? `对标 ${latest.benchmark}` : "按市场价估") : "估价走势"}
      onClose={onClose}
    >
      {body}
    </ChartDialog>
  );
}
```

(不足两天时仍显示范围按钮会更好用: 若实现时觉得 `rows.length < 2` 只在所选范围内不足而全量足够, 把范围按钮挪到 `body` 外、加载成功即显示; 测试里的「不足两天」用的是全量只有 1 天, 两种写法都满足。)

- [ ] **Step 4: GREEN**

Run: `cd doudou_blend && npx vitest run src/CoalPriceDialog.test.tsx`
Expected: 通过。

---

### Task 8: 卡片估价行可点 + 煤池接线 + 样式

**Files:** Modify `doudou_blend/src/CoalPriceLine.tsx`, `doudou_blend/src/CoalPriceLine.test.tsx`, `doudou_blend/src/screens/CoalPoolScreen.tsx` (`CoalCard` 约 370 行、卡片网格约 307 行), `doudou_blend/src/screens/CoalPoolScreen.test.tsx`, `doudou_blend/src/App.css`

- [ ] **Step 1: 写失败测试**

`CoalPriceLine.test.tsx` 加:

```tsx
  it("给了 onOpen 时是按钮, 点击只触发 onOpen 不冒泡; 文案不变", () => {
    const onOpen = vi.fn();
    const onCard = vi.fn();
    const { getByRole } = render(
      <div onClick={onCard}>
        <CoalPriceLine name="临北" price={{ kind: "snapshot", price: 2214, change: 0, missing: [] }} onOpen={onOpen} />
      </div>,
    );
    const button = getByRole("button", { name: "查看临北估价走势" });
    expect(button.textContent).toBe("估价 2,214 元/吨  持平");
    fireEvent.click(button);
    expect(onOpen).toHaveBeenCalledOnce();
    expect(onCard).not.toHaveBeenCalled();
  });

  it("估价不可用时即使给了 onOpen 也不是按钮", () => {
    const { queryByRole } = render(<CoalPriceLine name="临北" price={{ kind: "unavailable" }} onOpen={() => {}} />);
    expect(queryByRole("button")).toBeNull();
  });
```

(文件顶部 import 补 `fireEvent`、`vi`。)

`CoalPoolScreen.test.tsx` 文件末尾加一个 describe (复用文件里的 `master` / `snapshot` / `value` / `reply` 与
`beforeEach` / `afterEach`; 顶部 import 补 `fireEvent`。编辑器 `CoalEditor` 没有 `role="dialog"`, 用它独有的
「…保证值」输入框判断它没打开):

```tsx
describe("煤池估价走势", () => {
  it("点共享煤的估价行打开走势弹层, 不打开编辑器; Esc 关闭; 自建煤不可点", async () => {
    addUserCoal({ name: "自建煤", status: "verified", props: { S: 1.4 } });
    const history = { ok: true, name: "共享煤", points: [
      { day: "2026-10-27", price: 2190.1, benchmark: "基准", source: "backfill" },
      { day: "2026-10-28", price: 2202.4, benchmark: "基准", source: "live" },
    ] };
    vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/api/coal-prices") return reply(snapshot);
      if (url.startsWith("/api/coal-prices/history?")) return reply(history);
      const { coals } = JSON.parse(init!.body as string) as { coals: Coal[] };
      return reply({ ok: true, coal_values: coals.map((coal) => value(coal.name, 2180)) });
    }));
    render(<CoalPoolScreen />);
    const open = await screen.findByRole("button", { name: "查看共享煤估价走势" });
    await waitFor(() => expect(card("自建煤").textContent).toContain("估价 2,180 元/吨"));
    expect(screen.queryByRole("button", { name: "查看自建煤估价走势" })).toBeNull();
    fireEvent.click(open);
    expect(await screen.findByRole("dialog", { name: "共享煤" })).toBeTruthy();
    expect(screen.queryByLabelText(/保证值/)).toBeNull();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
```

- [ ] **Step 2: 跑测试确认 RED**

Run: `cd doudou_blend && npx vitest run src/CoalPriceLine.test.tsx src/screens/CoalPoolScreen.test.tsx`
Expected: 新增几条 FAIL, 原有通过。

- [ ] **Step 3: 实现**

`CoalPriceLine.tsx`: 现有渲染保持为内部的 `text` (原样), 新增可选参数:

```tsx
export function CoalPriceLine({ price, name, onOpen }: {
  price: CardPrice;
  /** 与 onOpen 一起给: 按钮的无障碍名 "查看{name}估价走势". */
  name?: string;
  /** 给了就变成按钮, 点开走势; 点击不冒泡到卡片 (卡片自己点开的是编辑器). */
  onOpen?: () => void;
}) {
```

`kind === "unavailable"` 照旧返回灰字 `<div>`; 否则若有 `onOpen`, 用
`<button type="button" className="coal-price-line" aria-label={`查看${name}估价走势`} onClick={(event) => { event.stopPropagation(); onOpen(); }}>`
包住原来那行的内容 (原 `<div style=...>` 的样式挪到按钮上, 文字内容一字不改); 没有 `onOpen` 时输出与现在完全相同。

`CoalPoolScreen.tsx`:

```tsx
import { CoalPriceDialog } from "../CoalPriceDialog";
import { cardPrice, coalsToEstimate, estimateLive, fetchCoalPrices, hasPriceHistory, isModified, type CardPrice } from "../coalPrices";
...
  const [chartCoal, setChartCoal] = useState<ResolvedCoal | null>(null);
...
            <CoalCard
              key={`${coal.origin}:${coal.name}:${index}`}
              coal={coal}
              price={cardPrice(coal, snapshot, currentLive)}
              onClick={() => setEditing(coal)}
              onOpenPrice={hasPriceHistory(coal, snapshot) ? () => setChartCoal(coal) : undefined}
            />
...
      {chartCoal && (
        <CoalPriceDialog
          name={chartCoal.name}
          modified={isModified(chartCoal)}
          onClose={() => setChartCoal(null)}
        />
      )}
```

`CoalCard` 加 `onOpenPrice?: () => void` 参数, 末尾改为 `<CoalPriceLine price={price} name={coal.name} onOpen={onOpenPrice} />`。

`App.css` (放在 `.index-chart-empty` 规则之后):

```css
.coal-price-line {
  display: block;
  width: 100%;
  margin: 8px 0 0;
  padding: 0;
  border: 0;
  background: none;
  font: inherit;
  font-size: 12px;
  color: var(--c-text-2);
  text-align: left;
  cursor: pointer;
}

.coal-price-line::after {
  content: " 走势 ›";
  color: var(--c-text-3);
}

.price-chart-note {
  margin: 8px 0 0;
  font-size: 12px;
  color: var(--c-text-3);
}
```

(`::after` 不进 `textContent`, 文案测试不受影响。)

- [ ] **Step 4: GREEN + 全量前端**

Run: `cd doudou_blend && npm test && npm run build`
Expected: 全部通过, 构建成功。

---

### Task 9: 文档、全量验证、真实 PostgreSQL 联调

**Files:** Modify `CLAUDE.md:68-69`

- [ ] **Step 1: 改 `CLAUDE.md`** 那两行为:

```markdown
`blend_kit_server/src/coal_prices.rs` 在服务启动后每小时重算当天 (北京时间) 的共享煤库估价快照, 并在启动时与每过零点
按"当天能拿到的指数与报价" (`benchmarks::as_of`) 补算今天以前缺的 (日期, 煤) —— 存下就不改, 只补空 (`source` 区分
live / backfill); `GET /api/coal-prices` 返回最新日与前一日价格, `GET /api/coal-prices/history?name=` 返回单煤全部历史;
煤池卡片据此显示估价与涨跌, 点估价行看走势; 用户修改或新增的煤按当前值现算.
```

- [ ] **Step 2: 全量验证** (每条记录结尾输出)

```bash
cd blend_kit_server && cargo fmt && cargo clippy --release -- -D warnings && cargo test --locked
cd ../doudou_blend && npm test && npm run build && npm run check:consistency && npm run check:data
```

Expected: 全部通过 (若 CLI 的端口监听测试因沙箱失败, 记下, 与基线对比)。

- [ ] **Step 3: 真实 PostgreSQL 联调** (docker `postgres:17-alpine`, 用完删容器; 照上一期计划的做法)

1. 起库, 以 `MCCI_FETCH=off` 与本地 `AUTH_*` / `DATABASE_URL` 启动一次服务让迁移跑完, 停掉;
2. 用 `psql` 往 `mcci_daily` 写两条序列 (如 `LJM_DLZJM`、`LJM_FM`) 从"今天 − 10 天"到"今天"的日度值; 往
   `coal_price_daily` 写昨天一种煤的一行 (`source` 不写, 价格写一个特殊值如 1.0);
3. 再启动服务, 看日志有「历史估价补算完成」, `days` ≈ 10, `rows` ≈ 种数 × 10 − 1;
4. 查: 昨天那行仍是 1.0 且 `source = live` (没被覆盖); 其余行 `source = backfill`; 今天的行全是 `live`;
   没有任何 `day >= 今天` 的 `backfill` 行;
5. `curl` (带登录 cookie) `GET /api/coal-prices/history?name=<某煤>`: 升序、带 `source`; 不存在的煤返回空 `points`;
6. 重启服务: 日志里补算 `rows = 0` (幂等);
7. 删掉容器。

- [ ] **Step 4: 自查审查重点** 逐条对照本计划开头的「审查重点」, 在下面的「实施记录」写结论。

---

## 实施记录

(实施者在此逐任务记录: RED/GREEN 的测试数、偏离计划之处与理由、联调结果、未完成项。)
