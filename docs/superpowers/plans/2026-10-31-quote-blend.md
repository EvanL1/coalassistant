# 询价精算 + 行情条常驻 实施计划

> **For agentic workers:** 使用 superpowers:executing-plans 在本工作区逐个任务实施; 每个任务严格 TDD (先写失败测试并跑出 RED,
> 再实现到 GREEN)。用户已批准设计, 不再请求设计批准。步骤用 `- [ ]` 勾选记录。

**Goal:** 今日页加「询价精算」: 用户当场填几种煤的出厂价与运费 (可改这批指标), 只用这几种煤精算; 每次精算把询价连同
服务端重算的同口径估价存档, 后台按煤种对比"报价 / 估价"; 行情条在今日页与煤池页顶部常驻。

**Architecture:** 核心 `blend_kit_rs` 不改 —— 精算请求把填了价的煤放进 `coals` (带 fob/frt), `extra_coals` 留空、
`estimate_unpriced` 关掉, 核心 `price_factor::attach` 自然给出 `coal_values[].gap` (报价 − 估价)。服务端新模块
`quotes.rs` 负责存档 (校验 → `fill_price_request` + `estimate_json` 重算估价 → 一个事务里按批覆盖) 与后台对比;
前端把今日页的请求拼装与配方/指标卡抽出来共用, 新增精算页 (在今日标签内原地替换今日页内容)。

**Tech Stack:** Rust / Axum 0.8 / SQLx 0.8 / PostgreSQL; React 19 / TypeScript / Vitest。

**Spec:** `docs/superpowers/specs/2026-10-30-quote-blend-design.md` (先通读; 其中「已定口径」四条是用户逐条确认的, 不得改口径)

---

## 全局约束

- 在分支 `feat/quote-blend` 的当前工作区实施; **不 commit、不 push**; 不动未跟踪的 `research/`。
- 不改核心 `blend_kit_rs` 与 master 数据 (`coal_master.json`); Rust 测试只用 fixture, 不断言 master 内容。
- 注释、日志、界面文案用中文, 标点跟随所在文件 (Rust 注释多为半角标点)。
- 每个 Rust 任务后跑 `cd blend_kit_server && cargo test --locked <过滤词>`; 每个前端任务后跑
  `cd doudou_blend && npx vitest run <文件>`。
- 现有测试一律不改期望。特别是: `doudou_blend/src/screens/TodayScreen.test.tsx` 全部原样通过 (抽组件、抽请求拼装
  都不能改它的期望); `doudou_blend/src/IndexTicker.test.tsx` 的「指数走势弹层」一节一行不改。
- 新的跨端结构 (Rust 序列化 ↔ `types.ts`) 都登记到 `scripts/check_repository_consistency.mjs`。

## 审查重点 (实施时自查, 交付后我会逐条审)

1. 精算请求: `coals` 只含填了出厂价 (> 0) 的煤, 每条 `fob` = 出厂价、`frt` = 运费 (空 = 0); `price_factor.extra_coals`
   为 `[]`、`estimate_unpriced` 为 `false`; 少于 2 种不发请求。不读煤池/偏好里的任何价格键。
2. 询价**不写**煤池、偏好、localStorage、cloudStorage; 当场改的指标只进本次请求与存档。下次进精算页输入框为空,
   上次询价只作 `placeholder`。
3. 存档估价只由服务端算, 忽略浏览器传来的任何估价字段; 只认"按基准估出来的价" —— 核心返回的该煤 `benchmark` 为空时
   `estimate` 记 NULL (没基准时核心会拿这批报价自己反推市场价, 那是循环论证)。估价按出厂口径算 (运费不进估价)。
4. 同一 `(username, batch_id)` 按批覆盖: DELETE + INSERT 在**一个事务**里, 失败不留半批。
5. 存档失败或接口不可用时, 精算结果照常显示, 只多一行"询价未存档: 原因"; 精算失败 (不可行) 时存档照做。
6. 鉴权与检查顺序 (都有路由测试):
   - `POST /api/quotes`: 401 → 415 (非 JSON) → 400 (请求体 / 校验 / 核心估价报错) → 503。
   - `GET /api/quotes/latest`: 401 → 503。
   - `GET /api/admin/quotes`: 401 / 403 → 400 (limit 非法) → 503。
   - `DELETE /api/admin/quotes/{id}`: 401 / 403 → 503 → 404 (非法或不存在的 id)。
7. 今日页现有行为不变; 行情条在今日与煤池两页渲染并 sticky, 走势弹层 (z-index 1000) 盖在它上面。
8. "到厂价低于 X 才会配进去" 只对**不在今日配方里**且 `blend_value` 为有限正数的煤显示。

## 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `doudou_blend/src/App.tsx` / `App.css` | 改 | 行情条在今日、煤池两页渲染; sticky |
| `blend_kit_server/migrations/0013_coal_quotes.sql` | 新 | `coal_quotes` 表 |
| `blend_kit_server/src/quotes.rs` | 新 | 校验、估价、按批覆盖写库、最近询价、后台明细/汇总/删除 |
| `blend_kit_server/src/lib.rs` | 改 | `mod quotes;` + 4 条路由 + 路由测试 |
| `scripts/check_repository_consistency.mjs` | 改 | 登记 `ArchivedQuote` / `LatestQuote` / `QuoteRecord` / `QuoteTypeSummary` |
| `doudou_blend/src/types.ts` | 改 | 上面 4 个结构 + 响应外壳; `HistoryRecord.quoted` |
| `doudou_blend/src/quotes.ts` (+test) | 新 | `archiveQuotes` / `fetchLatestQuotes` |
| `doudou_blend/src/admin.ts` (+test) | 改 | `fetchQuoteComparison` / `deleteQuote` |
| `doudou_blend/src/domain/blendRequest.ts` (+test) | 新 | 今日页与精算页共用的请求拼装 |
| `doudou_blend/src/screens/BlendCards.tsx` | 新 | 从今日页抽出的配方卡 `RecipeCard`、指标卡 `IndicatorCard` |
| `doudou_blend/src/screens/TodayScreen.tsx` | 改 | 用上面两处抽取; 加「询价精算」入口与 `quoting` 状态 |
| `doudou_blend/src/screens/QuoteScreen.tsx` (+test) | 新 | 精算页 |
| `doudou_blend/src/backend.ts` (+test) | 改 | 历史 DTO 派生 `quoted` |
| `doudou_blend/src/screens/HistoryScreen.tsx` (+test) | 改 | 「询价」小标 |
| `doudou_blend/src/screens/admin/QuoteCompareCard.tsx` (+test) | 新 | 后台「询价对比」 |
| `doudou_blend/src/screens/AdminScreen.tsx` | 改 | 挂上新卡片 |
| `CLAUDE.md` | 改 | 估价/询价口径段落 |

---

### Task 1: 行情条常驻 (今日 + 煤池)

**Files:** Modify `doudou_blend/src/App.tsx`, `doudou_blend/src/App.css`; Test: 新建 `doudou_blend/src/App.test.tsx`
(若 App 因登录门挂载成本过高, 可改为把"哪些标签显示行情条"抽成导出常量 `TICKER_TABS` 并对它与 `IndexTicker`
的渲染位置写测试; 两种都不行就在实施记录里说明, 改由 Task 12 的浏览器验证覆盖)。

- [x] **Step 1: 失败测试** —— 登录后切到「煤池」标签, 页面里有行情条 (`IndexTicker` 的根元素, 可用其 `aria-label`
  或 `.index-ticker` 查询); 切到「合同」没有。`fetchMcci` 照 `IndexTicker.test.tsx` 的方式 mock。
- [x] **Step 2: 跑出 RED**: `npx vitest run src/App.test.tsx`。
- [x] **Step 3: 实现**

`App.tsx`:
```tsx
          {tab === "today" && (
            <>
              <IndexTicker />
              <TodayScreen onNavigate={setTab} />
            </>
          )}
          {tab === "pool" && (
            <>
              <IndexTicker />
              <CoalPoolScreen />
            </>
          )}
```
`App.css` (`.index-ticker` 基础规则里加; 宽屏 `@media (min-width: 720px)` 里已有 `.index-ticker` 的那段加 background):
```css
.index-ticker {
  /* 行情常驻: 页面是 window 滚动, sticky 直接生效; 底色盖住滚上来的内容. 走势弹层 z-index 1000 在它之上. */
  position: sticky;
  top: 0;
  z-index: 10;
  background: var(--c-bg);
  ...原有声明不动
}

@media (min-width: 720px) {
  .index-ticker {
    margin-bottom: 6px;
    padding-inline: 0 !important;
    background: #eef1f5; /* 与宽屏 body 底色一致 */
  }
}
```
- [x] **Step 4: GREEN** + `npx vitest run src/IndexTicker.test.tsx` 原样通过。

---

### Task 2: 迁移 + `quotes.rs` 请求校验 (纯函数)

**Files:** Create `blend_kit_server/migrations/0013_coal_quotes.sql`, `blend_kit_server/src/quotes.rs`;
Modify `blend_kit_server/src/lib.rs` (`mod quotes;`)

- [x] **Step 1: 迁移** (照抄, 不改列名):
```sql
-- 询价存档: 用户当场询来的出厂价与运费, 附服务端按这批指标 + 当天基准重算的同口径估价.
-- 只用于对比与以后校准估价, 不进煤池、不跨次沿用. 同一批 (batch_id) 多次精算以最后一次为准.
CREATE TABLE IF NOT EXISTS coal_quotes (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    quoted_on DATE NOT NULL,
    name TEXT NOT NULL,
    coal_type TEXT,
    form TEXT,
    region TEXT,
    fob DOUBLE PRECISION NOT NULL CHECK (fob > 0),
    frt DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (frt >= 0),
    props JSONB NOT NULL,
    changed JSONB NOT NULL DEFAULT '[]',
    estimate DOUBLE PRECISION,
    points DOUBLE PRECISION,
    benchmark TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (username, batch_id, name)
);
CREATE INDEX IF NOT EXISTS coal_quotes_day ON coal_quotes (quoted_on);
CREATE INDEX IF NOT EXISTS coal_quotes_user_name ON coal_quotes (username, name, created_at);
```

- [x] **Step 2: 失败测试** (`quotes.rs` 里 `#[cfg(test)] mod tests`):
```rust
fn item(name: &str, fob: f64) -> QuoteItem {
    QuoteItem {
        name: name.into(),
        fob,
        frt: 0.0,
        coal_type: Some("主焦煤".into()),
        form: None,
        region: None,
        props: serde_json::from_value(json!({ "S": 0.6, "A": 9.8 })).unwrap(),
        changed: vec![],
    }
}

fn batch(items: Vec<QuoteItem>) -> QuoteBatch {
    QuoteBatch { batch_id: "0b6f4c1e-1d7a-4c55-9a51-2f0f6f3f1a10".into(), items }
}

#[test]
fn test_validate_accepts_a_normal_batch() {
    assert_eq!(validate(&batch(vec![item("临北", 2150.0), item("沙曲", 1980.0)])), Ok(()));
}

#[test]
fn test_validate_rejects_bad_batches() {
    let mut bad_id = batch(vec![item("临北", 2150.0)]);
    bad_id.batch_id = "../x".into();
    let mut bad_frt = item("临北", 2150.0);
    bad_frt.frt = -1.0;
    let mut bad_changed = item("临北", 2150.0);
    bad_changed.changed = vec!["X".into()];
    for (case, expect) in [
        (bad_id, "batch_id"),
        (batch(vec![]), "1–50"),
        (batch((0..51).map(|i| item(&format!("煤{i}"), 2000.0)).collect()), "1–50"),
        (batch(vec![item("临北", 2150.0), item("临北", 2100.0)]), "重复"),
        (batch(vec![item("  ", 2150.0)]), "煤名"),
        (batch(vec![item("临北", 0.0)]), "出厂价"),
        (batch(vec![item("临北", f64::NAN)]), "出厂价"),
        (batch(vec![item("临北", 100_000.0)]), "出厂价"),
        (batch(vec![bad_frt]), "运费"),
        (batch(vec![bad_changed]), "X"),
    ] {
        let error = validate(&case).unwrap_err();
        assert!(error.contains(expect), "{error}");
    }
}

#[test]
fn test_batch_ignores_client_estimate_fields() {
    let parsed: QuoteBatch = serde_json::from_value(json!({
        "batch_id": "abc-1",
        "items": [{ "name": "临北", "fob": 2150, "props": {}, "estimate": 1, "gap": 2 }]
    }))
    .unwrap();
    assert_eq!(parsed.items[0].frt, 0.0);
}
```
- [x] **Step 3: RED**: `cargo test --locked quotes::` (编译失败即 RED)。
- [x] **Step 4: 实现** (模块头 + 类型 + `validate`):
```rust
//! 询价存档: 询价精算时用户当场填的出厂价与运费 (每次精算存一次, 同一批以最后一次为准),
//! 附服务端按这批指标 + 线上当前基准重算的同口径估价; 后台按煤种对比"报价 / 估价".
//!
//! 只用于对比与以后校准估价 —— 不进煤池、不跨次沿用 (当场有效).

const MAX_QUOTE_BODY: usize = 256 * 1024;
const MAX_ITEMS: usize = 50;
const MAX_NAME_CHARS: usize = 100;
const MAX_FOB: f64 = 100_000.0;
const MAX_FRT: f64 = 10_000.0;

/// 一次精算的询价. 不拒收未知字段: 浏览器顺手带上的估价之类一律忽略, 估价只由服务端算.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QuoteBatch {
    pub batch_id: String,
    pub items: Vec<QuoteItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QuoteItem {
    pub name: String,
    /// 出厂价 (含税), 元/吨.
    pub fob: f64,
    /// 运费, 元/吨; 不填记 0.
    #[serde(default)]
    pub frt: f64,
    #[serde(default)]
    pub coal_type: Option<String>,
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    /// 这批实际用的指标 (煤池值 ∪ 当场改动).
    pub props: Map<String, Value>,
    /// 当场改过的指标名.
    #[serde(default)]
    pub changed: Vec<String>,
}

fn validate(batch: &QuoteBatch) -> Result<(), String> {
    let id = &batch.batch_id;
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("batch_id 须为 1–64 位字母、数字或连字符".into());
    }
    if batch.items.is_empty() || batch.items.len() > MAX_ITEMS {
        return Err(format!("询价须为 1–{MAX_ITEMS} 条"));
    }
    let mut names = HashSet::new();
    for item in &batch.items {
        let name = item.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("煤名不能为空, 且不超过 {MAX_NAME_CHARS} 字"));
        }
        if !names.insert(name) {
            return Err(format!("煤名重复: {name}"));
        }
        if !(item.fob.is_finite() && item.fob > 0.0 && item.fob < MAX_FOB) {
            return Err(format!("{name} 的出厂价须在 0–{MAX_FOB:.0} 元之间"));
        }
        if !(item.frt.is_finite() && item.frt >= 0.0 && item.frt < MAX_FRT) {
            return Err(format!("{name} 的运费须在 0–{MAX_FRT:.0} 元之间"));
        }
        if let Some(bad) = item.changed.iter().find(|key| !INDICATORS.contains(&key.as_str())) {
            return Err(format!("{name} 的改动指标 {bad} 不认识"));
        }
    }
    Ok(())
}
```
(`INDICATORS` 用 `blend_kit::INDICATORS`。)
- [x] **Step 5: GREEN**。

---

### Task 3: 服务端估价 (纯函数)

**Files:** Modify `blend_kit_server/src/quotes.rs`

- [x] **Step 1: 失败测试** —— 用 fixture master (`price_factor` 带 1 条 S 规则 + 1 条主焦煤基准; 写法参照
  `coal_prices.rs` / `benchmarks.rs` 测试里已有的 master fixture), 不用 `coal_master.json`:
```rust
#[test]
fn test_estimate_items_prices_each_quote_from_benchmarks() {
    let master = fixture_master(); // 一条基准: 主焦煤, S 0.6 → 2000 元
    let benchmarks = vec![]; // 空 = 用 master 自带基准 (fill_price_request 的既有行为)
    let estimates = estimate_items(&[item("临北", 2150.0)], &master, &benchmarks).unwrap();
    assert_eq!(estimates.len(), 1);
    let fair = estimates[0].fair.unwrap();
    assert!((fair - 2000.0).abs() < 1e-6, "{fair}"); // 指标与基准一致 → 估价 = 基准价
    assert_eq!(estimates[0].benchmark.as_deref(), Some(/* fixture 基准名 */ "测试主焦"));
}

#[test]
fn test_estimate_ignores_freight() {
    let master = fixture_master();
    let mut with_frt = item("临北", 2150.0);
    with_frt.frt = 300.0;
    let a = estimate_items(&[item("临北", 2150.0)], &master, &[]).unwrap();
    let b = estimate_items(&[with_frt], &master, &[]).unwrap();
    assert_eq!(a, b);
}

#[test]
fn test_estimate_without_benchmark_is_none() {
    // master 里没有任何基准 (也没有 market_price): 核心会拿这批报价反推市场价, 不能当估价.
    let estimates = estimate_items(&[item("临北", 2150.0)], &fixture_master_without_benchmarks(), &[]).unwrap();
    assert_eq!(estimates[0].fair, None);
}

#[test]
fn test_estimate_reports_core_validation_errors() {
    let mut bad = item("临北", 2150.0);
    bad.props.insert("A".into(), json!(-5)); // 核心 validate_coal 会拒绝的值, 以真实规则为准调整
    assert!(estimate_items(&[bad], &fixture_master(), &[]).is_err());
}
```
- [x] **Step 2: RED**。
- [x] **Step 3: 实现**:
```rust
#[derive(Debug, Clone, PartialEq, Default)]
struct Estimate {
    /// 同口径 (出厂) 估价; 没对上基准为 None.
    fair: Option<f64>,
    points: Option<f64>,
    benchmark: Option<String>,
}

/// 按线上当前系数表与基准估这批煤的出厂价. 只认按基准估出来的价: 没对上基准时核心会拿
/// 这批报价自己反推市场价, 拿它和报价比是循环论证.
fn estimate_items(items: &[QuoteItem], master: &Value, benchmarks: &[Value]) -> Result<Vec<Estimate>, String> {
    let mut request = json!({ "coals": items.iter().map(price_coal).collect::<Vec<_>>() });
    benchmarks::fill_price_request(&mut request, master, benchmarks);
    let output: Value = serde_json::from_str(&blend_kit::estimate_json(&request.to_string()))
        .map_err(|error| format!("估价结果解析失败: {error}"))?;
    if output["ok"] != json!(true) {
        return Err(output["reason"].as_str().unwrap_or("估价失败").to_string());
    }
    let values = output["coal_values"].as_array().cloned().unwrap_or_default();
    Ok(items
        .iter()
        .map(|item| {
            let Some(value) = values.iter().find(|value| value["name"] == item.name.as_str()) else {
                return Estimate::default();
            };
            let benchmark = value["benchmark"].as_str().map(str::to_string);
            Estimate {
                fair: value["fair"]
                    .as_f64()
                    .filter(|fair| fair.is_finite() && *fair > 0.0 && benchmark.is_some()),
                points: value["points"].as_f64(),
                benchmark,
            }
        })
        .collect())
}

/// 估价用的煤: 出厂口径 (运费不进), 空值指标去掉 (同每日快照).
fn price_coal(item: &QuoteItem) -> Value {
    let props: Map<String, Value> = item
        .props
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut coal = json!({ "name": item.name, "props": props, "fob": item.fob, "frt": 0 });
    for (key, value) in [("coal_type", &item.coal_type), ("form", &item.form), ("region", &item.region)] {
        if let Some(value) = value {
            coal[key] = json!(value);
        }
    }
    coal
}
```
- [x] **Step 4: GREEN**。

---

### Task 4: 写库 + `POST /api/quotes` + `GET /api/quotes/latest`

**Files:** Modify `blend_kit_server/src/quotes.rs`, `blend_kit_server/src/lib.rs`

- [x] **Step 1: 失败的路由测试** (`lib.rs` 的 `mod tests`, 用现成的 `test_app()` / `session_request` / `response_text`;
  若 `session_request` 不带 Content-Type, 照它的写法加一个带 `application/json` 的变体):
```rust
#[tokio::test]
async fn test_archive_quotes_checks_login_type_body_then_database() {
    let valid = r#"{"batch_id":"abc-1","items":[{"name":"临北","fob":2150,"props":{"S":0.6}}]}"#;
    for (token, json_type, body, expected) in [
        ("forged", true, valid, StatusCode::UNAUTHORIZED),
        ("test-session-token", false, valid, StatusCode::UNSUPPORTED_MEDIA_TYPE),
        ("test-session-token", true, "not json", StatusCode::BAD_REQUEST),
        ("test-session-token", true, r#"{"batch_id":"abc-1","items":[]}"#, StatusCode::BAD_REQUEST),
        ("test-session-token", true, valid, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        // 发 POST /api/quotes, json_type 决定是否带 Content-Type: application/json
        // 断言状态码与 body.ok == false
    }
}

#[tokio::test]
async fn test_latest_quotes_requires_login_and_database() {
    // forged → 401; test-session-token → 503
}
```
- [x] **Step 2: RED**。
- [x] **Step 3: 实现**

DTO (都 `Serialize`, 登记到一致性脚本):
```rust
/// 存档后回给精算页的一条: 报价与服务端估价的对比.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchivedQuote {
    pub name: String,
    pub fob: f64,
    pub estimate: Option<f64>,
    /// fob − estimate.
    pub gap: Option<f64>,
    /// fob / estimate.
    pub ratio: Option<f64>,
    pub benchmark: Option<String>,
}

/// 本账号某种煤最近一次询价, 只作精算页输入框的占位提示.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct LatestQuote {
    pub name: String,
    pub fob: f64,
    pub frt: f64,
    pub quoted_on: NaiveDate,
}
```
写库:
```rust
/// 按批覆盖: 同一 (账号, 批) 先删后插, 一个事务, 失败不留半批.
async fn replace_batch(
    pool: &PgPool,
    username: &str,
    day: NaiveDate,
    batch: &QuoteBatch,
    estimates: &[Estimate],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM coal_quotes WHERE username = $1 AND batch_id = $2")
        .bind(username)
        .bind(&batch.batch_id)
        .execute(&mut *tx)
        .await?;
    for (item, estimate) in batch.items.iter().zip(estimates) {
        sqlx::query(
            r#"
            INSERT INTO coal_quotes
                (id, username, batch_id, quoted_on, name, coal_type, form, region,
                 fob, frt, props, changed, estimate, points, benchmark)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(username)
        .bind(&batch.batch_id)
        .bind(day)
        .bind(&item.name)
        .bind(&item.coal_type)
        .bind(&item.form)
        .bind(&item.region)
        .bind(item.fob)
        .bind(item.frt)
        .bind(Value::Object(item.props.clone()))
        .bind(json!(item.changed))
        .bind(estimate.fair)
        .bind(estimate.points)
        .bind(&estimate.benchmark)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}
```
handler:
```rust
/// POST /api/quotes —— 存档一批询价 (登录即可). 估价由这里重算, 不信任浏览器.
pub(crate) async fn archive(State(state): State<AppState>, headers: HeaderMap, request: Request<Body>) -> Response {
    let username = match authenticated_username(&headers, &state) {
        Ok(username) => username,
        Err(response) => return *response,
    };
    if let Err(response) = require_json_content_type(&headers) {
        return *response;
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_QUOTE_BODY).await else {
        return bad_request("请求体读取失败或超出大小上限");
    };
    let batch: QuoteBatch = match serde_json::from_slice(&body) {
        Ok(batch) => batch,
        Err(error) => return bad_request(&format!("询价格式无效: {error}")),
    };
    if let Err(reason) = validate(&batch) {
        return bad_request(&reason);
    }
    let master: Value = serde_json::from_str(&merged_master_json(&state).await).unwrap_or_default();
    let benchmarks = benchmarks::current(&state, &master).await;
    let estimates = match estimate_items(&batch.items, &master, &benchmarks) {
        Ok(estimates) => estimates,
        Err(reason) => return bad_request(&reason),
    };
    let pool = match require_database(&state) {
        Ok(pool) => pool,
        Err(response) => return *response,
    };
    let day = Utc::now().with_timezone(&mcci::beijing()).date_naive();
    match replace_batch(pool, &username, day, &batch, &estimates).await {
        Ok(()) => Json(json!({ "ok": true, "items": archived(&batch.items, &estimates) })).into_response(),
        Err(error) => database_error(error),
    }
}
```
`archived` 是纯函数 (单测: gap/ratio 算对, estimate 为 None 时两者为 None)。

`GET /api/quotes/latest`:
```sql
SELECT DISTINCT ON (name) name, fob, frt, quoted_on
FROM coal_quotes
WHERE username = $1
ORDER BY name, created_at DESC
```
返回 `{ "ok": true, "quotes": [...] }`。

路由 (`lib.rs`, 放在 `/api/coal-prices/history` 附近):
```rust
        .route("/api/quotes", post(quotes::archive))
        .route("/api/quotes/latest", get(quotes::latest))
```
`authenticated_username` / `require_json_content_type` / `merged_master_json` 是 `lib.rs` 的私有函数, 子模块可直接
`use super::{...}` 引用 (`admin.rs` 就是这么做的)。
- [x] **Step 4: GREEN**: `cargo test --locked quotes` 与 `cargo test --locked test_archive_quotes test_latest_quotes`。

---

### Task 5: 后台明细 / 汇总 / 删除

**Files:** Modify `blend_kit_server/src/quotes.rs`, `blend_kit_server/src/lib.rs`

- [x] **Step 1: 失败测试**

纯函数:
```rust
fn record(coal_type: Option<&str>, fob: f64, estimate: Option<f64>) -> QuoteRecord { /* 其余字段给定值 */ }

#[test]
fn test_summarize_groups_by_type_with_medians() {
    let rows = vec![
        record(Some("主焦煤"), 2200.0, Some(2000.0)), // 1.10, +200
        record(Some("主焦煤"), 2100.0, Some(2000.0)), // 1.05, +100
        record(Some("主焦煤"), 2400.0, Some(2000.0)), // 1.20, +400
        record(Some("肥煤"), 1900.0, Some(2000.0)),   // 0.95, -100
        record(Some("肥煤"), 2100.0, Some(2000.0)),   // 1.05, +100
        record(None, 1500.0, Some(1500.0)),
        record(Some("肥煤"), 1800.0, None),            // 没估价, 不进汇总
    ];
    let summary = summarize(&rows);
    assert_eq!(summary[0].coal_type, "主焦煤");
    assert_eq!(summary[0].count, 3);
    assert!((summary[0].median_ratio - 1.10).abs() < 1e-9);
    assert!((summary[0].median_gap - 200.0).abs() < 1e-9);
    assert_eq!(summary[1].coal_type, "肥煤");
    assert_eq!(summary[1].count, 2);
    assert!((summary[1].median_ratio - 1.0).abs() < 1e-9); // 偶数取中间两个的平均
    assert!((summary[1].median_gap - 0.0).abs() < 1e-9);
    assert_eq!(summary[2].coal_type, "未标煤种");
}
```
路由 (`lib.rs`):
```rust
#[tokio::test]
async fn test_admin_quotes_guards() {
    // GET /api/admin/quotes: forged → 401; test-session-token → 403; 管理员 + ?limit=abc → 400; 管理员 → 503
    // DELETE /api/admin/quotes/x: forged → 401; test-session-token → 403; 管理员 → 503
}
```
- [x] **Step 2: RED**。
- [x] **Step 3: 实现**
```rust
const DEFAULT_LIMIT: i64 = 200;
const MAX_LIMIT: i64 = 1000;
const UNTYPED: &str = "未标煤种";

/// 后台明细一行.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct QuoteRecord {
    pub id: String,
    pub username: String,
    pub batch_id: String,
    pub quoted_on: NaiveDate,
    pub name: String,
    pub coal_type: Option<String>,
    pub form: Option<String>,
    pub region: Option<String>,
    pub fob: f64,
    pub frt: f64,
    #[sqlx(json)]
    pub props: Map<String, Value>,
    #[sqlx(json)]
    pub changed: Vec<String>,
    pub estimate: Option<f64>,
    pub points: Option<f64>,
    pub benchmark: Option<String>,
    pub created_at: DateTime<Utc>,
    #[sqlx(skip)]
    pub gap: Option<f64>,
    #[sqlx(skip)]
    pub ratio: Option<f64>,
}

/// 按煤种汇总 "报价 / 估价": 取中位数, 个别手误带不偏.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuoteTypeSummary {
    pub coal_type: String,
    pub count: usize,
    pub median_ratio: f64,
    pub median_gap: f64,
}
```
- `summarize(&[QuoteRecord]) -> Vec<QuoteTypeSummary>`: 只算 `estimate > 0` 的行; 空或全空白煤种记 `UNTYPED`;
  按条数降序、同条数按煤种名升序。
- 汇总基于**全部**有估价的行 (单独一条查询), 不受明细 `limit` 截断。
- `list` handler: `require_admin` → 解析 `limit` (缺省 200; 非整数或 < 1 → 400 "limit 须为正整数"; 大于 1000 截到 1000)
  → `require_database` → 明细 `ORDER BY created_at DESC LIMIT $1` (查完填 `gap` / `ratio`) + 汇总 → 返回
  `{ "ok": true, "quotes": [...], "by_type": [...] }`。
- `delete` handler: 照抄 `benchmarks::delete` 的结构 (`require_admin` → `require_database` → `Uuid::parse_str` 失败 → 404
  → `DELETE FROM coal_quotes WHERE id = $1`, 0 行 → 404), 成功 `{ "ok": true }`。`not_found` 若在 `benchmarks.rs` 是私有的,
  就提到 `lib.rs` 共用或在本模块写同样的一个, 二选一, 不要复制粘贴两份逻辑不同的。

路由:
```rust
        .route("/api/admin/quotes", get(quotes::list))
        .route("/api/admin/quotes/{id}", delete(quotes::delete))
```
- [x] **Step 4: 一致性登记** —— `scripts/check_repository_consistency.mjs` 照 `CoalPricePoint` 那两行, 登记
  `ArchivedQuote` / `LatestQuote` / `QuoteRecord` / `QuoteTypeSummary` (新增 `rustQuotesSource` 指向 `quotes.rs`);
  `types.ts` 加同名 interface (字段一一对应, `NaiveDate` / `DateTime` → `string`, `Option<T>` → `T | null`)。
- [x] **Step 5: GREEN**: `cargo test --locked`; `cd doudou_blend && npm run check:consistency`。

---

### Task 6: 前端接口 (`quotes.ts`, `admin.ts`)

**Files:** Create `doudou_blend/src/quotes.ts` + `quotes.test.ts`; Modify `doudou_blend/src/admin.ts` + `admin.test.ts`

- [x] **Step 1: 失败测试** (mock `fetch`, 写法照 `coalPrices.test.ts`):
  - `archiveQuotes(batchId, items)`: POST `/api/quotes`, `Content-Type: application/json`, body 原样; 返回 `ArchivedQuote[]`;
    非 2xx 时抛出带服务端 `reason` 的 `Error` (精算页据此显示"询价未存档: 原因"); 返回形状不对也抛。
  - `fetchLatestQuotes()`: GET `/api/quotes/latest`; 失败或形状不对返回空 `Map` (只是占位提示, 不该报错);
    返回 `Map<name, LatestQuote>`。
  - `fetchQuoteComparison()`: GET `/api/admin/quotes`; 返回 `{ quotes, by_type }`; 形状不对抛。
  - `deleteQuote(id)`: DELETE `/api/admin/quotes/${encodeURIComponent(id)}`。
  (`admin.ts` 里若已有统一的请求/报错封装, 两个新函数沿用它。)
- [x] **Step 2: RED → Step 3: 实现 → Step 4: GREEN**。

`quotes.ts` 里定义精算页提交用的条目类型 (不是 Rust 镜像, 不登记一致性):
```ts
/** 精算时存档的一条询价: 出厂价 + 运费 + 这批实际用的指标. */
export interface QuoteItemInput {
  name: string;
  fob: number;
  frt: number;
  coal_type?: string;
  form?: string;
  region?: string;
  props: Partial<Record<Indicator, number | null>>;
  changed: Indicator[];
}
```
(`Indicator` 用 `types.ts` 里已有的指标名类型; 没有就用 `INDICATORS` 常量推导。)

---

### Task 7: 抽出共用的请求拼装 `domain/blendRequest.ts`

**Files:** Create `doudou_blend/src/domain/blendRequest.ts` + `blendRequest.test.ts`; Modify `TodayScreen.tsx`

目标: `TodayScreen.runSolve` 里 "基准 → 价格因子 → 请求骨架" 那段 (现 `TodayScreen.tsx` 约 309–334 行) 抽成纯函数 +
一个异步取价函数, 今日页与精算页共用。今日页的"用户输入在任何 await 前冻结"约束保持不变 (调用方先冻结输入再调)。

- [x] **Step 1: 失败测试**
```ts
it("今日估价请求: 煤全在 extra_coals, 打开 estimate_unpriced", () => {
  const request = estimateRequest(base, [coalA, coalB]);
  expect(request.coals).toEqual([]);
  expect(request.price_factor?.extra_coals).toEqual([coalA, coalB]);
  expect(request.price_factor?.estimate_unpriced).toBe(true);
});

it("询价精算请求: 只含填了价的煤, extra_coals 为空, 关掉 estimate_unpriced", () => {
  const request = quoteRequest(base, [
    { coal: resolvedA, fob: 2150, frt: 80, props: {} },
    { coal: resolvedB, fob: 1980, frt: 0, props: { S: 1.1 } },
  ]);
  expect(request.coals.map((c) => [c.name, c.fob, c.frt])).toEqual([["A", 2150, 80], ["B", 1980, 0]]);
  expect(request.coals[1].props.S).toBe(1.1);          // 当场改的指标进本次请求
  expect(resolvedB.props.S).not.toBe(1.1);             // 不改煤池对象
  expect(request.price_factor?.extra_coals).toEqual([]);
  expect(request.price_factor?.estimate_unpriced).toBe(false);
  expect(request.shovel_tons).toBe(base.shovelTons);
  expect(request.specs).toBe(base.specs);
});
```
- [x] **Step 2: RED → Step 3: 实现**, API 形如:
```ts
/** 今日页与询价精算共用的请求骨架: 合同、吨数、一铲、价格因子 (系数表 + 当前基准). */
export interface BlendBase {
  specs: Spec[];
  totalQuantity: number;
  shovelTons: number | null;
  /** solverPriceFactor + 当前基准 (或市场价); 不含 extra_coals / estimate_unpriced. */
  priceFactor: PriceFactor;
}

/** 取当前基准 (取不到退回 master 默认, 都没有才用现货指数点位). 现 TodayScreen 那段原样搬来. */
export async function loadPriceFactor(priceFactor: MasterPriceFactor): Promise<PriceFactor>;

export function estimateRequest(base: BlendBase, coals: Coal[]): BlendRequest;

/** 精算用的一种煤: 煤池指标 ∪ 本次改动, 带出厂价与运费; 扣款条款同今日页. */
export interface QuotedCoal {
  coal: ResolvedCoal;
  fob: number;
  frt: number;
  props: Partial<Record<Indicator, number>>;
  purchaseTerms?: PurchaseTerms | null;
}

export function quoteRequest(base: BlendBase, quoted: QuotedCoal[]): BlendRequest;
```
- [x] **Step 4: 改 `TodayScreen.runSolve` 用它们**; `npx vitest run src/screens/TodayScreen.test.tsx` 原样通过 (测试里对
  `fetchBenchmarks` / `fetchLatestCoalIndex` 的 mock 仍要生效 —— `blendRequest.ts` 从 `../benchmarks` / `../coal_index`
  引入同名函数即可)。

---

### Task 8: 抽出配方卡与指标卡 `screens/BlendCards.tsx`

**Files:** Create `doudou_blend/src/screens/BlendCards.tsx`; Modify `TodayScreen.tsx`

- [x] **Step 1**: 把今日页 `.today-recipe` 卡 (配方条 + 列表) 与 `.today-indicators` 卡原样搬成
  `RecipeCard({ result, ... })` / `IndicatorCard({ result, ... })` (props 以实际需要为准, 不新增外观); 今日页改用它们。
- [x] **Step 2**: `npx vitest run src/screens/TodayScreen.test.tsx` 原样通过 (这一步是纯搬家, 现有测试就是它的测试)。

---

### Task 9: 精算页 `QuoteScreen` + 今日页入口

**Files:** Create `doudou_blend/src/screens/QuoteScreen.tsx` + `QuoteScreen.test.tsx`; Modify `TodayScreen.tsx` (+ test), `App.css`

入口: 今日页 `.action-row` 加按钮「询价精算」, 有快照 (含不可行结果) 时显示, `refreshing` 时禁用。点它 `setQuoting(true)`,
今日页原地渲染 `<QuoteScreen todayResult={snapshot.result} onBack={() => setQuoting(false)} />` (今日页其余内容不渲染,
今日页状态保留; 返回后看到的还是原来的估价结果)。

精算页行为 (每条都要有测试, mock 写法照 `TodayScreen.test.tsx` 顶部):
1. 挂载时 `batchId = crypto.randomUUID()` (测试里 stub), 自己 `loadMaster` + `resolveCoalPool` 取煤池 (启用且 `readiness === "ready"`
   的煤可选), `fetchLatestQuotes()` 取占位提示。
2. 默认列出今日配方里的煤 (从 `todayResult` 取, 顺序同今日页配方); 下拉框可加煤池里其余可选煤; 每行可去掉。
3. 每行: 煤名、今日估价 (`todayResult.coal_values` 里该煤的 `fair`, 取整显示)、出厂价输入、运费输入、「改这批指标」展开。
   - 出厂价/运费输入框为空; 有上次询价时 `placeholder` = `上次 2150 (10-28)` 之类, 不作为值。
   - 不在今日配方里、且 `blend_value` 为有限正数: 灰字 `到厂价低于 {round(blend_value)} 元才会配进去`。
   - 出厂价与今日估价偏离 > 30% (`QUOTE_DEVIATION_WARN = 0.3`): 行内提示 `比今日估价高/低 N%, 请核对`, 不拦。
   - 展开后 8 项指标 (`INDICATORS` 顺序) 各一个输入框, 默认煤池值; 改了的标出来; 空 = 用煤池值; 非数字视为未改并提示。
4. 填了有效出厂价 (> 0) 的煤 < 2 种: 「精算」禁用, 提示 `至少填两种煤的出厂价`。
5. 点「精算」: 用 `quoteRequest` 拼请求 (只含填了价的煤) 求解; **同时** `archiveQuotes(batchId, items)` (`items` = 同一批填了价的煤,
   `props` 为这批实际用的指标, `changed` 为改过的指标名)。两者互不阻塞: 存档失败只显示 `询价未存档: 原因`。同一会话多次精算
   用同一个 `batchId`。用 `LatestRequestTracker` (今日页在用) 防止旧结果覆盖新结果。
6. 结果: 可行时 `CostCard` + `RecipeCard` + `IndicatorCard` + 「报价对比」表 (煤 · 用量 · 出厂报价 · 估价 · 差额, 差额 > 0 记
   `高 X 元`, < 0 记 `低 X 元`; 数据取结果的 `coal_values` 的 `fob` / `fair` / `gap`, 只列这批的煤); 不可行时用 `InfeasiblePanel`。
7. 「保存到历史」: `backend.saveHistory(result, contractName, totalQuantity)`, 与今日页保存同样的成功/失败文案风格。
8. 「← 返回今日」调用 `onBack`。
9. 询价数据不写任何本地存储 (测试: 精算前后 `localStorage` 不变; `storage` 模块的写函数未被调用)。

- [x] **Step 1: 按上面 1–9 写失败测试** (一条一个 `it`), 加今日页测试: 「询价精算」按钮出现、点开后渲染精算页、返回后仍是原估价结果。
- [x] **Step 2: RED → Step 3: 实现 → Step 4: GREEN**。样式写进 `App.css` (类名前缀 `quote-`), 手机宽度不出横向滚动。

---

### Task 10: 历史「询价」标记

**Files:** Modify `doudou_blend/src/types.ts` (`HistoryRecord.quoted: boolean`), `backend.ts` (+ `backend.test.ts`),
`screens/HistoryScreen.tsx` (+ test)

- [x] **Step 1: 失败测试**: `listHistory` 对 `result.coal_values` 里任一 `fob` 为正数的记录给 `quoted: true`, 否则 (含无 result、
  `coal_values` 缺失或损坏) `false`; 历史页对 `quoted` 记录显示 `询价` 小标 (用现有 `.badge`)。
- [x] **Step 2: RED → Step 3: 实现 → Step 4: GREEN** (其他使用 `HistoryRecord` 的测试夹具补上 `quoted: false`)。

---

### Task 11: 后台「询价对比」卡

**Files:** Create `doudou_blend/src/screens/admin/QuoteCompareCard.tsx` + test; Modify `screens/AdminScreen.tsx`

- [x] **Step 1: 失败测试**: 加载中/失败/空 (`还没有询价`) 三态; 汇总表 (煤种 · 条数 · 报价/估价中位 (如 `1.10`) · 差额中位
  (`+200`)); 明细表 (日期 · 煤 · 煤种 · 对标基准 · 出厂 · 运费 · 估价 · 差额 · 比例 · 改过的指标), 估价为 null 显示 `—`;
  每行「删除」先 `confirm("删除这条询价? 删除后不进汇总")`, 确认才调 `deleteQuote`, 成功后重新加载。
  卡片风格照 `screens/admin/CsrModelCard.tsx` 与 `admin/styles.ts`。
- [x] **Step 2: RED → Step 3: 实现 (挂到 `AdminScreen`, 放在 `BenchmarkCard` 之后) → Step 4: GREEN**。

---

### Task 12: 文档、全量校验、联调

- [x] **Step 1: `CLAUDE.md`**: 把 "Estimate-only pricing" 段改为: 求解默认全按估价; 唯一例外是今日页「询价精算」—— 用户当场
  填几种煤的出厂价与运费, 只用这几种煤求解, 询价不进煤池、不跨次沿用; 每次精算按批存档到 `coal_quotes` (估价由
  `blend_kit_server/src/quotes.rs` 用当天基准重算), 后台「询价对比」按煤种汇总, 供以后校准估价系数。同时在架构段
  `coal_prices.rs` 那句后补一句 `quotes.rs` 的职责。
- [x] **Step 2: 全量**:
```bash
cd blend_kit_server && cargo fmt && cargo clippy --release -- -D warnings && cargo test --locked
cd doudou_blend && npm test && npm run build && npm run check:consistency && npm run check:data
```
- [ ] **Step 3: 真实 PostgreSQL 联调** (需要 docker; 沙箱里跑不了就跳过并在实施记录里说明, 我在本机做):
  起一个 PG, `DATABASE_URL` 指过去启动服务, 用测试账号登录后:
  1. POST 一批 2 条 → 库里 2 行, `estimate` 非空, `quoted_on` 为北京日期;
  2. 同 `batch_id` 再 POST 1 条 → 库里只剩 1 行;
  3. 换 `batch_id` POST → 两批并存; `GET /api/quotes/latest` 每煤只回最新一条;
  4. 管理员 `GET /api/admin/quotes` 有明细与汇总; `DELETE` 一条后汇总条数减 1; 再删同 id → 404。
- [ ] **Step 4: 本机浏览器** (`npm run dev` + 本地服务): 今日页 → 询价精算 → 填两种煤 → 精算 → 保存 → 历史页有「询价」;
  今日页与煤池页往下滚, 行情条停在顶上; 点行情条品种, 走势弹层盖在它上面。

---

## 实施记录

(实施者逐 Task 填写: RED/GREEN 测试数、偏离计划之处与理由。)

- 执行约定: 使用本节作唯一实施台账, 测试日志放 `/tmp/quote-blend-checks/`; 按用户要求留在当前分支, 不新建工作树、不提交.
- 接口预检: Task 2–5 的 DTO → Task 6/11 的接口与后台; Task 7/8 的请求与卡片 → Task 9; Task 9 的结果 → Task 10 的派生标记. 无口径冲突.
- Task 1: complete. RED: App 1 失败 (煤池无行情条); GREEN: App + IndexTicker 2 文件、7 测试通过. 保留指数弹层测试原文. 无偏离.
- Task 2: complete. RED: 缺少 QuoteBatch/QuoteItem/validate, 编译失败; GREEN: 3 测试通过. 迁移列名按计划. 首次命令误在仓库根目录运行, 随即在 server crate 重跑取得真实 RED.
- Task 3: complete. RED: estimate_items 缺失编译失败; GREEN: 7 测试通过 (新增 4). fixture 使用真实 S/Upper 系数结构, A=-5 确认触发核心校验. 无口径偏离.
- Task 4: complete. RED: archived 缺失; 路由另跑得到 2 失败 (405/404 而非 401); GREEN: quotes 过滤 12 测试通过 (新增 3). Ruling: 原计划一个 cargo test 带两个过滤词不合法, 合并为 cargo test --locked quotes, 覆盖两个路由与纯函数.
- Task 5: complete. RED: QuoteRecord/summarize/parse_limit 缺失编译失败; GREEN: server 库 134 测试通过、1 忽略, CLI 8 通过、1 受沙箱监听限制失败 (`test_two_profiles_against_live_server`, Operation not permitted), 未改该测试. 一致性检查通过. 新增空煤种/无估价/limit 边界覆盖.
- Task 6: complete. RED: quotes 模块缺失 + admin 6 失败; GREEN: 2 文件、28 测试通过. API 同时校验条目字段, 损坏响应不会进入页面. Indicator 使用仓库既有 IndicatorKey.
- Task 7: complete. RED: blendRequest 模块缺失; GREEN: 请求 4 + TodayScreen 原有 28 = 32 测试通过. Ruling: npm exec 实际使用 Node v26.13.0, 其内置 localStorage 遮蔽 jsdom, 导致 28 测试在 beforeEach 的 clear 失败; 仅命令设置 NODE_OPTIONS=--no-experimental-webstorage 后全绿, 不改既有测试或配置. 后续前端验证沿用该环境变量.
- Task 8: complete. 按计划纯抽取, 用 Task 7 已通过的 28 项 TodayScreen 测试作基线; 抽取后原样 28/28 通过, 不人为制造 RED. 配方与指标 JSX/文案/判定保持原样.
- Task 9: complete. RED: QuoteScreen 模块缺失; GREEN: QuoteScreen 17 + TodayScreen 原有 28 = 45 通过. 新增入口测试放在 QuoteScreen.test.tsx, TodayScreen.test.tsx 完全不改. Ruling: 同一会话的存档顺序发送, 求解独立并行, 防旧请求最后写库覆盖新价; 存档卡住时后续存档会排队, 结果不受影响. 构建发现新 fixture 的 map 推断过窄, 显式指定 MasterCoalEntry 类型修正构造, 断言不变.
- Task 10: complete. RED: 8 失败、9 通过; GREEN: backend + HistoryScreen 共 17 通过. 仅补旧历史 fixture 的 quoted: false, 既有期望不变; 损坏 coal_values 与非数字 fob 不会误标.
- Task 11: complete. RED: QuoteCompareCard 缺失; GREEN: 6 测试通过 (加载、空、失败、数值、删除确认/刷新/失败). 后台放在 BenchmarkCard 后.
- Task 12: 文档完成; 第一轮全量: cargo fmt 无输出成功, cargo clippy --release -- -D warnings 成功 (`Finished release profile`, 2.16s); cargo test --locked: 库 134 passed/1 ignored, CLI 8 passed/1 failed (src/bin/doudou.rs:599, test_two_profiles_against_live_server, PermissionDenied: Operation not permitted), 退出码 101. 按用户授权只记录, 不改端口测试.
- Task 12: 前端使用 `NODE_OPTIONS=--no-experimental-webstorage`: npm test 为 38 文件/408 测试通过; npm run build 为 81 modules transformed, built in 831ms; check:consistency 通过 (112 条, v0.1.1); check:data 通过 (112 条), 保留既有 1 条煤种判据和 24 种地区匹配提示, 未改 master.
- Task 12 Step 3/4: 跳过真实 PostgreSQL 与本地服务浏览器联调. `docker info --format '{{.ServerVersion}}'` 返回 docker.sock permission denied; CLI 监听测试证明沙箱不允许启动测试服务. 用户在本机按上面的联调清单验收; jsdom 组件测试不能代替真实滚动、遮挡和 PostgreSQL 事务验证.
- Task 1 补充 Ruling: 计划把宽屏底色规则写成 ≥720px, 真实仓库中 body 的 #eef1f5 与该行情条规则都在 ≥960px; 在原规则处加背景, 720–959px 保持 var(--c-bg), 使常驻条底色跟随实际页面. 代价: 与设计文字的断点数值不同, 手机和宽屏视觉均仍需浏览器确认.
- Final review: 一次独立只读审查, 发现 3 项 Important、0 Critical、0 Minor. 无基准对比与不可行 warnings 已由新增测试重现: RED 2 失败/17 通过 → GREEN 19/19; 既有成功对比 fixture 补上真实基准字段, 原断言含义未改.
- Final: fixed 同批并发覆盖竞态: DELETE 前新增按 (username, batch_id) 的事务级 advisory lock, 防止两个并发事务删到旧快照后留下并集. 新增 quotes_db_tests.rs 的隔离 schema 并发回归测试, 通过 pg_stat_activity 确认后到请求已等待后才提交先到请求.
- Final: Ruling: PostgreSQL 并发回归无法实际跑 RED/GREEN (docker.sock 被沙箱拒绝), 测试仅编译并标记 ignored, 不能算通过. 按用户允许跳过真实 PG 联调的授权完成静态修复; 风险是数据库锁/事务行为仍需本机验证. 本机命令: 在 server crate 下 `DATABASE_URL=独立测试库 cargo test --locked test_replace_quote_batch_serializes_concurrent_writers -- --ignored`.
- Final: Ruling: 审查列出的其余行为保持现状: 存档队列与输入失效防旧结果覆盖; 非法/空指标回退与最近询价静默失败符合设计; 成功结果不额外展示 warnings 跟随今日页; 960px 底色跟随实际 CSS; 全量汇总查询适合本期规模, 未加依赖. 代价: 存档挂起会使后续存档排队, 成功结果普通 warnings 不展示, 大量历史样本时汇总查询需再评估.
- Final validation: 修复后再次跑完全部指定命令. cargo fmt exit 0 (无输出); cargo clippy --release -- -D warnings exit 0 (`Finished release profile`, 2.85s); cargo test --locked exit 101: 库 134 passed/0 failed/2 ignored (既有外网 MCCI + 新增本机 PG 并发测试), CLI 8 passed/1 failed, 唯一失败仍为端口监听权限. PostgreSQL 回归初次编译遇到 sqlx raw_sql 在 after_connect 中的生命周期限制, 改用绑定参数的 set_config 初始化连接后编译通过, 未改变测试断言.
- Final validation: `NODE_OPTIONS=--no-experimental-webstorage` 下 npm test exit 0 (38 文件/410 passed), npm run build exit 0 (81 modules, built in 475ms), check:consistency exit 0 (112 条, 0.1.1), check:data exit 0 (112 条, 既有数据提示不变). `git diff --check` 通过. core/WASM/TodayScreen.test.tsx/IndexTicker.test.tsx 的 diff 全为空; HEAD 不变, 未提交、未推送、未触碰 research/. 无暂缓 Minor.

### 审查 (Claude, 2026-10-31)

- 修 bug: 精算页挂在今日页下, 今日页后台重算 (改煤池、云同步) 失败会切到错误页, 把精算页连同已填的报价一起卸掉。
  改为 `quoting` 存进入时的估价结果, 在 loading/error 判断之前渲染精算页。补回归测试「精算页开着时今日页后台重算失败,
  已填的报价不丢」: 旧写法下 RED (1 失败), 修后 GREEN。
- 可读性: `QuoteScreen.tsx` 拆出 `QuoteRowCard` / `QuoteComparison` / `formatGap`, `QuoteCompareCard.tsx` 拆出
  `HeaderRow` / `DetailRow`, 都改成逐行展开的 JSX; 询价 CSS 按文件既有风格逐条展开; `blendRequest.ts` 补回从今日页
  搬走时丢掉的说明注释 (基准回退顺序、一铲、extra_coals 与 estimate_unpriced 的含义); `quotes.rs` 补项间空行。行为与文案不变。
- 精算页配方卡标题改为「精算配方」(`RecipeCard` 加可选 `title`, 今日页默认不变), 测试补断言。
- 本机联调 (docker postgres:16):
  - `test_replace_quote_batch_serializes_concurrent_writers -- --ignored` 通过。
  - 接口: 同批两次提交 2 行 → 1 行 (2120); 浏览器带的 `estimate` 被忽略, 服务端按这批指标重算并记基准; 换批并存,
    `/api/quotes/latest` 每煤一条; 后台明细/汇总正确, 删除 200 → 再删 404, 汇总条数随之减 1; 普通账号调后台 403。
  - 浏览器 (宽屏): 今日页 → 询价精算默认带出今日配方 4 种煤与今日估价, 上次询价只作占位; 偏离 50% 行内提示不拦;
    精算 1969.04 元/吨 (含运费), 报价对比 4 行; 保存后历史有「询价」标记; 库里这批 4 行, 估价与页面一致;
    今日页与煤池页滚动时行情条停在顶上, 走势弹层盖在其上。手机宽度未在真机上看。
- 全量: server fmt/clippy (`--all-targets`) 通过, 库 134 passed + CLI 9 passed (本机无沙箱限制, 端口测试也过);
  前端 Node 22: 38 文件 / 411 passed, build、check:consistency 通过。
