# 每日估价快照 + 煤池卡片显示估价 设计

日期: 2026-10-28
状态: 已批准 (用户确认: 每日快照 + 卡片价; 全局快照, 改过的煤现算; 方案 A 每小时重算今天)

## 目的

煤价一律按估算 (见 PR #16)。估价模块 (`blend_kit_rs/src/pricing.rs`) 能给每种煤估价，但目前:

- 没有任何地方按日把估价记下来 —— 基准 (MCCI 每晚入库、管理员补录) 天天在变, 每种煤哪天值多少钱无从回查;
- 煤池卡片只显示 8 项指标, 估价只在求解结果里、且只对参与求解的煤可见。

本期: **服务端每天给共享煤库里的每种煤估价并存库** (攒出每种煤的日度估价序列, 数据资产),
**煤池卡片显示估价与较前一日涨跌**。

## 已定的口径

1. **快照是全局的**: 按合并后的共享煤库 (master + 管理员覆盖层) 估, 每种煤一条序列, 不按用户分。
2. **煤的范围**: 共享煤库里**全部**的煤, 含已停用 (archived) 与数据不全 (incomplete) 的 —— 停用的是暂时不进,
   不等于没有市场价; 缺的指标记入 `missing`, 卡片上提示。
3. **估价口径与今日页求解完全一致**: 系数表 = master `price_factor` 去掉 `MASTER_ONLY_KEYS`;
   基准 = `benchmarks::current` (master ∪ 补录 ∪ MCCI 最新值, 旧报价按同煤种指数推算到今天)。
   所以一种没改过的煤, 卡片价 = 求解时用的估算价 (采购扣款另算, 不在估价里)。
4. **用户改过化验值或自己新增的煤**: 卡片按该用户的值现算 (`POST /api/price`), 标「按修改值估」,
   不显示涨跌 (快照里的序列是共享煤库的值, 拿来比没有意义)。

## 生成方式: 独立后台任务, 每小时重算"今天"

- 服务启动、连上库后起后台任务 (与 `mcci::spawn_daily` 并列, 但**不受 `MCCI_FETCH` 开关影响**:
  MCCI 关了, master 与补录的基准照样会变)。`tokio::time::interval` 首次立即触发 → 启动即算一次。
- 每小时: 读合并后的煤库与当前基准 → 估价 → 在一个事务里**删掉今天 (北京时间日期) 的整份, 再整份写入**。
- 过了北京时间零点, 昨天那份不再被改写 → 定格为"当天最后一次估价" (含当晚 17:30 后入库的 MCCI)。
- 每一轮放在子任务里跑, panic 只丢这一轮并记日志 (照 `mcci::spawn_daily` 的写法)。
- 估价失败 (系数表非法、核心返回 `ok: false`) 或读写库失败: 只记错误日志, 库里旧数据不动。

为什么不挂在 MCCI 取数成功之后: 取数失败、停更或关掉的日子历史会断档; 管理员白天补录的基准、
改的煤库也进不了当天快照。为什么不在打开煤池时懒生成: 没人看的日子就没有记录。

## 存储

新迁移 `blend_kit_server/migrations/0011_coal_price_daily.sql`:

```sql
-- 每种煤每天的估价快照 (见 src/coal_prices.rs): 服务端每小时重算当天 (北京时间) 的整份,
-- 过了零点定格. 按共享煤库 (master + 管理员覆盖层) 估, 不含用户自己改的化验值.
-- 同时记下对标的基准与基准价: 将来分得清价格变动是行情动了还是换了对标基准. 整段历史留着.
CREATE TABLE IF NOT EXISTS coal_price_daily (
    day DATE NOT NULL,
    name TEXT NOT NULL,
    price DOUBLE PRECISION NOT NULL,          -- 估价 (元/吨, 即核心的 fair)
    points DOUBLE PRECISION NOT NULL,         -- 质量指数 (标准品 1000 点)
    benchmark TEXT,                           -- 对标的基准名; 按市场价估时为 NULL
    benchmark_price DOUBLE PRECISION,         -- 该基准当时的生效价 (current_price, 没有则 price)
    missing JSONB NOT NULL DEFAULT '[]',      -- 缺指标而跳过的规则
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (day, name)
);
CREATE INDEX IF NOT EXISTS coal_price_daily_name_day ON coal_price_daily (name, day);
```

量级: 112 种 × 365 天 ≈ 4 万行/年, 不清理。

## 服务端组件

### 小重构: 让后台任务能拿到"合并煤库"与"当前基准"

`merged_master_json(&AppState)` 与 `benchmarks::current(&AppState, ..)` 实际只用到 `state.database`。
各加一个只收 `Option<&PgPool>` 的版本 (原函数改为薄包装, 现有调用方不变), 后台任务只持有 `PgPool`。

### 新模块 `blend_kit_server/src/coal_prices.rs`

纯函数 (无 IO, 单测覆盖):

- `beijing_day(now: DateTime<Utc>) -> NaiveDate` —— 北京时间的日期。
- `snapshot_rows(master: &Value, benchmarks: &[Value]) -> Result<Vec<PriceRow>, String>` ——
  组请求 `{"coals": [...]}` (每种煤: name、非 null 的 props、fob/frt = 0、coal_type/form/region),
  用 `benchmarks::fill_price_request` 补系数表与基准, 调 `blend_kit::estimate_json`;
  `ok: false` → `Err(reason)`; 否则逐个 `coal_values` 映射成行, `benchmark_price` 按基准名回查
  (`current_price` 优先, 没有取 `price`)。
- `compose(latest: Option<DaySnapshot>, prev: Option<DaySnapshot>) -> Value` —— 组接口响应, 按煤名配上前一日价;
  `DaySnapshot { day, computed_at, rows: Vec<PriceRow> }`, `computed_at` 取该日各行的最大值。

IO:

- `write_snapshot(pool, day, rows)` —— 事务内 `DELETE WHERE day = $1` 再批量 `INSERT` (UNNEST 一条语句)。
- `refresh(pool)` —— 合并煤库 + 当前基准 → `snapshot_rows` → `write_snapshot`。
- `spawn_hourly(pool)` —— 在 `lib.rs::app` 里有库时起 (不看 `MCCI_FETCH`)。
- `list` handler —— `GET /api/coal-prices`。

### 接口 `GET /api/coal-prices` (登录即可)

响应沿用本项目现有的 `{ ok, ... }` 形状 (与 `/api/benchmarks`、`/api/mcci` 一致):

```json
{
  "ok": true,
  "day": "2026-10-28",
  "prev_day": "2026-10-27",
  "computed_at": "2026-10-28T09:00:03Z",
  "prices": [
    { "name": "临北", "price": 2202.4, "prev_price": 2190.1, "points": 1083.2,
      "benchmark": "安泽低硫主焦", "missing": [] }
  ]
}
```

- `day` = 库里最新的一天; `prev_day` = 早于它的最近一天 (没有为 `null`, 对应各煤 `prev_price` 省略)。
- 还没有任何快照: `{"ok": true, "day": null, "prices": []}`。
- 没配库: 与其他读库接口一致 (`require_database` 的 503)。
- 命令行不另加子命令, `doudou api GET /api/coal-prices` 即可。

## 前端

### 数据层 `doudou_blend/src/coalPrices.ts`

- `fetchCoalPrices(): Promise<CoalPriceSnapshot | null>` —— 取不到 (离线、服务异常) 返回 `null`。
- `estimateLive(coals: Coal[]): Promise<Map<string, CoalValue> | null>` —— 一次 `POST /api/price`
  把需要现算的煤打包估价; 失败返回 `null`。
- 纯函数 `cardPrice(coal: ResolvedCoal, snapshot, live): CardPrice` 决定每张卡显示什么:
  - 用户新增 (`origin === "user"`)、有化验值覆盖 (`overriddenProps` 非空)、或快照里没有这种煤 → 走现算:
    `{ kind: "live", price, missing }`; 现算没结果 → `{ kind: "unavailable" }`;
  - 否则 `{ kind: "snapshot", price, change, missing }`, `change = price − prev_price` (没有前一日为 `null`);
  - `invalid_override` 的煤不现算 → `{ kind: "unavailable" }`。
- 需要现算的煤: 由 `ResolvedCoal` 直接拼 `Coal` (props/coal_type/form/region, fob/frt = 0) ——
  不复用 `toEstimateCoal`, 因为它只放行 `ready` 的煤, 而卡片要给停用的煤也标价。
- 新 DTO (`CoalPriceSnapshot`、`CoalPriceEntry`) 写进 `types.ts`, 与 Rust 响应字段一致。

### 煤池卡片 `CoalPoolScreen.tsx`

- 进入煤池时并行取快照、对需要现算的煤调一次 `/api/price` (煤池每次切 tab 都重挂载, 数据跟着刷新);
  用户在编辑框改了化验值 (`doudou:prefs_changed`) 后重新现算。
- 卡片 8 项指标下方加一行 (组件 `CoalPriceLine`):
  - 快照: `估价 2,202 元/吨  ▲12` / `▼8` / `持平`; 涨跌不上颜色 (卡片上绿色已表示"启用中", 再用红绿表涨跌会混);
  - 现算: `估价 2,180 元/吨 · 按修改值估`;
  - 缺指标: 价后加 `· 缺 Y` (多项用顿号连接);
  - 不可用: `估价暂不可用` (灰字), 卡片其余照常。
- 页面副标题追加 `· 估价 10-28` (快照日), 没快照时不显示。
- 价格取整到元, 千分位。

## 测试 (TDD)

Rust (`blend_kit_server`, 只用 fixture, 不断言 master 内容):

- `beijing_day`: UTC 15:59 / 16:00 跨日边界。
- `snapshot_rows`: fixture 煤库 (含一种 archived、一种缺 Y) + 一条基准 → 每种煤一行;
  `benchmark` / `benchmark_price` 取对 (有 `current_price` 时用它); 缺 Y 的 `missing == ["Y"]`;
  系数表非法 → `Err`。
- `compose`: 有/无前一日; 前一日里没有的煤不带 `prev_price`。
- 接口: 未登录 401; 没配库 503 (照现有 handler 测试写法)。

前端 (vitest):

- `cardPrice` 的四种情况 (快照含涨跌、快照无前一日、现算、不可用) 与"改过化验值即走现算"。
- `fetchCoalPrices` / `estimateLive` 失败返回 `null`。
- `CoalPriceLine` 渲染: 涨/跌/持平、按修改值估、缺指标、暂不可用。

同步: `npm run check:consistency` 通过 (新 DTO 与 Rust 字段一致); 文档 `CLAUDE.md` 架构段补一句快照任务与接口。

## 不做 (YAGNI)

- 单煤估价走势图 (数据已在库里, 需要时加接口参数即可);
- MCP 工具与 `doudou` 子命令带估价;
- 按用户的快照; 快照的清理或降采样。
