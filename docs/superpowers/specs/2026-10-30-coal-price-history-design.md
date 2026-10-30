# 每种煤的历史估价 (回算一年 + 走势图) 设计

日期: 2026-10-30
状态: 已批准 (用户确认: 按当天能拿到的算; 点煤池卡片看走势图; 存下就不改, 只补缺的天)

## 目的

每日估价快照 (PR #17, `coal_price_daily`) 从 2026-10-29 才开始攒, 而钢铁网指数 (`mcci_daily`) 已有
2025-10-27 起一整年。本期:

1. **按算法把每种煤过去一年的估价补算出来存库**, 与每天的实时快照放在同一张表里, 成为一条完整的日度序列;
2. **以后每天的估价照旧每小时算、过零点定格**, 并自动补齐任何缺的 (日期, 煤) —— 服务停机的日子、
   新加进煤库的煤, 都会补上;
3. **点煤池卡片上的估价行, 弹出这种煤一年的估价走势**。

## 已定的口径

1. **按当天能拿到的算 (point-in-time)**: 补算某天 D 时, 只用报价日 ≤ D 的基准 (master、补录表、指数三层都是)、
   ≤ D 的钢铁网指数与 CCP, 推算涨跌也只推到 D。等于假装系统一年前就上线了, 将来可以拿来回测。
   - 后果 (真实如此, 不修饰): 10-09 前没有 master 的 9 条主基准, 只靠 7 条指数对标; 贫瘦煤 (现有 5 种)
     没有同煤种的基准, 按 `pricing.rs` 的规则 (没有同煤种基准时不收窄) 对到升贴水最接近的别的基准,
     10-15 起才换成贫瘦煤报价 —— 这两处曲线会跳一下。
   - 煤的化验值与系数表 (`price_factor.adjusts`) 用现行的: 库里没有它们的历史版本。
2. **存下就不改, 只补缺的**: 已有的行永远不覆盖; 补算只填 (日期, 煤) 主键上还空着的格子。
   - 10-29 那份 (服务休眠, 按 10-28 的旧指数算的) 原样保留;
   - 以后改了算法或煤库数据, 历史不重算 (用户决定; 一条曲线上可能混着几代算法);
   - 新加进煤库的煤, 自动补上它自己过去一年的历史, 别的煤已定格的行不动。
3. **一天一个价**: 实时快照仍是"当天最后一次", 不存一天内每小时的价 (指数一天只出一次)。
4. **口径与实时快照同一套代码**: 实时快照 = "按今天能拿到的算"。补算与实时调用同一个函数, 不出现两套口径。

## 存储

新迁移 `blend_kit_server/migrations/0012_coal_price_source.sql`:

```sql
-- 估价快照的来历: live = 当天每小时重算、过零点定格的实时快照; backfill = 事后按"当天能拿到的
-- 指数与报价"补算的 (服务停机的日子、新加的煤、上线前的一年). 两种都存下就不改.
ALTER TABLE coal_price_daily
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'live'
    CHECK (source IN ('live', 'backfill'));
```

已有的行 (10-29、10-30 的实时快照) 自动是 `live`。量级: 112 种 × 约 368 天 (2025-10-27 至今) ≈ 4.1 万行, 不清理。

## 服务端

### `benchmarks.rs`: 把"当前生效基准"改成按日期取

现在 `current_from_pool` 隐含"今天": 取库表里每个名字最新的报价、指数序列的最新值、推算到指数最新一期。
拆成"读输入"和"纯函数算"两步:

- `struct BenchmarkInputs { stored: Vec<Value>, history: HashMap<String, Vec<IndexPoint>>, ccp: Option<Vec<IndexPoint>> }`
  —— `load_inputs(pool: Option<&PgPool>) -> (BenchmarkInputs, Option<String>)` 一次读齐, 读库失败的降级与
  报错方式照现在的 `current_from_pool` (记日志、退回 master 层、把错误交给后台任务)。
  - `stored` 改为库表**全部**报价 (不再 `DISTINCT ON`), 按 `name, quoted_on, created_at` **升序**排 ——
    `merge` 同日时后来者胜 (`date >= kept`), 这样同名同日仍是 `created_at` 最新的那条胜出, 与现在一致。
- 纯函数 `as_of(master: &Value, inputs: &BenchmarkInputs, day: NaiveDate) -> Vec<Value>`:
  1. master 基准与库表报价只留 `date <= day` 的;
  2. 每条指数序列、CCP 都截到 `date <= day`;
  3. 之后与现在完全相同: `index_benchmarks` → `merge(master, stored, indexed)` → `apply_drift(drift_series)`。
- `current_from_pool(pool, master)` 变成 `load_inputs` + `as_of(master, inputs, 北京时间今天)` 的薄包装,
  现有调用方 (`/api/price`、求解、实时快照) 不改。
  - 唯一的行为变化: 报价日写在未来的补录报价, 到那天才生效 (以前录进去立即生效)。

### `coal_prices.rs`: 补算

- `write_snapshot` (实时, 当天整份删了重写) 写入的行 `source = 'live'`, 其余不变。
- `backfill(pool, today) -> Result<BackfillReport, String>`:
  1. 读合并煤库与 `load_inputs` **一次**, 后面每天复用; 输入读不全 (库表或指数读库失败、CCP 取不到)
     → 整轮不写, 下个小时重试 —— 补下去就定格了, 不能拿缺料的结果填;
  2. 范围: 起点 = `max(今天 − 400 天, 最早一条可用输入的日期)` (400 天即 `mcci::HISTORY_DAYS`, 与指数读库的窗口一致; 输入 = 指数各序列首日、master 与库表报价日),
     终点 = 昨天 (北京时间) —— **永远不碰今天**, 今天归实时快照管;
  3. 一条查询读出范围内已有的 `(day, name)`; 只对还有缺的那些天算: `snapshot_rows(master, as_of(day))`,
     只留缺的煤;
  4. 批量 `INSERT ... ON CONFLICT (day, name) DO NOTHING`, `source = 'backfill'` (按若干天一批的 UNNEST,
     不用一个大事务 —— 每批本身幂等, 中途失败下次接着补);
  5. 某天 `as_of` 一个基准都没有 → 跳过这天 (不能交给 `snapshot_rows`: `fill_price_request` 在基准为空时会
     退回 master 自带的基准, 那里有晚于这天的报价, 等于偷看未来); 某天估价失败 (`snapshot_rows` 返回 `Err`)
     → 同样跳过; 跳过的天计入报告, 汇总记一条日志。
  - 估价是同步的 CPU 活 (约 368 天 × 一次 `estimate_json`), 放进 `tokio::task::spawn_blocking`, 不占异步线程。
- 调度: 并进现有的 `spawn_hourly` 循环。每一轮先 `refresh` (实时, 卡片价优先), 再看"上次补算成功的北京日期"
  是不是今天, 不是就跑 `backfill` —— 即**启动时一次 + 每过零点一次**; 补算失败不记日期, 下个小时重试。
  补算同样放在子任务里, panic 只丢这一轮。
- 零点竞态: 实时写入在提交前检查"还是不是今天", 跨日即回滚 (现有逻辑); 补算的终点是启动那一刻的昨天,
  且 `DO NOTHING` 不覆盖, 两者不会互相改写。

### 接口 `GET /api/coal-prices/history?name=临北` (登录即可)

```json
{
  "ok": true,
  "name": "临北",
  "points": [
    { "day": "2025-10-27", "price": 2050.3, "benchmark": "低硫主焦煤指数", "source": "backfill" },
    { "day": "2026-10-30", "price": 2214.0, "benchmark": "安泽低硫主焦", "source": "live" }
  ]
}
```

- 按 `day` 升序, 含今天的实时行; 走现有索引 `coal_price_daily_name_day`。
- 缺 `name` 或为空 → 400; 煤库里没有这种煤 → `{"ok": true, "name": ..., "points": []}`。
- 未登录 401; 没配库 503 (照现有 handler)。
- Rust 响应结构 `CoalPriceHistory { ok, name, points }`、`CoalPricePoint { day, price, benchmark, source }`
  写在 `coal_prices.rs`, 加进 `scripts/check_repository_consistency.mjs` 的 `sharedStructs`。
- 命令行不另加子命令: `doudou api GET '/api/coal-prices/history?name=临北'`。

## 前端

### 数据层

- `types.ts`: `CoalPricePoint { day; price; benchmark: string | null; source: "live" | "backfill" }`、
  `CoalPriceHistory { ok; name; points }`, 与 Rust 字段一致。
- `coalPrices.ts`: `fetchCoalPriceHistory(name): Promise<CoalPriceHistory | null>` —— 名字 URL 编码;
  取不到 (离线、服务异常、形状不对) 返回 `null`, 照 `fetchCoalPrices` 的写法。

### 走势图: 复用行情条的 `IndexChartDialog`

`doudou_blend/src/IndexChartDialog.tsx` 已有一套走势弹层: 弹层外壳 (遮罩、Esc 关闭、焦点进出)、
1月/3月/6月/1年切换、`PriceChart` (SVG 折线、自适应宽度、十字线 + tooltip、←/→/Home/End 逐日读)。
本期把其中通用的部分抽成共用组件, 指数弹层与煤价弹层都用它; **指数弹层的外观与现有测试不变**。

`PriceChart` 扩展 (都是可选参数, 不传即现在的样子):

- **虚实线**: 按点的 `dashed` 分段画, 回算段虚线 (`strokeDasharray`), 实时段实线; 相邻两段共用交界点, 线不断开。
- **基准变更标记**: 对标基准与前一天不同的那天, 在横轴上方画一个小竖标。
- **tooltip 附加行**: 煤价弹层显示 `对标 安泽低硫主焦` 与 `回算` (仅回算点)。

新组件 `CoalPriceDialog` (煤价弹层):

- 标题: 煤名; 副标题: `对标 {最新一天的基准}`;
- 摘要: 最新价 + 日期, 所选范围内的最高、最低;
- 范围切换与图同指数弹层 (默认 1年);
- 图下脚注: `虚线为回算: 按当天能拿到的指数和报价补算`;
  改过化验值的煤 (`isModified`) 再加一行 `历史为共享煤库口径, 不含你的修改`;
- 加载中 `加载中…`; 取不到 `走势暂时取不到`; 不足两天 `数据不足两天, 画不出走势`。

### 煤池卡片

- `CoalPriceLine` 加可选 `onOpen`: 有它时整行是一个按钮 (`aria-label="查看{煤名}估价走势"`),
  点击不冒泡到卡片 (卡片自己的点击行为不变)。
- 可点的条件: 共享煤库里有这种煤的快照 (`snapshot.prices` 里有这个名字), 即快照价、以及改过化验值的共享煤;
  用户自己新增的煤没有历史, 不可点。
- `CoalPoolScreen` 持有"打开的是哪种煤", 打开时才请求那一种煤的历史 (不预取 112 种)。

## 测试 (TDD)

Rust (`blend_kit_server`, 只用 fixture, 不断言 master 内容):

- `as_of` 不看未来: 在输入里加报价日晚于 D 的报价、晚于 D 的指数点与 CCP 点, `as_of(D)` 结果不变;
- `as_of` 截到当天: 某条指数 D 前后都有值时, 指数基准取 ≤ D 的最后一期, 旧报价的推算比例也只到 D;
- `as_of(今天)` 与改动前一致: 全部输入都不晚于今天时, 结果等于原来的 merge + drift;
- 同名同日的库表报价, `created_at` 最新的胜出 (排序改动的回归);
- 补算的纯逻辑: 给定已有的 `(day, name)` 与范围, 只算有缺的天、只留缺的煤, 不含今天;
- 范围起点: `max(今天 − 400, 最早输入日)`; 没有任何输入时范围为空;
- 某天估价失败被跳过并计数, 其余天照常;
- 接口: 未登录 401; 缺 `name` 400; 没配库 503; `compose` 风格的纯函数测历史响应 (升序、带 `source`)。

前端 (vitest):

- `fetchCoalPriceHistory`: 名字编码、失败返回 `null`;
- `PriceChart`: 不传新参数时与现在一致 (现有指数弹层测试原样通过); 回算段为虚线; 基准变更处有标记;
  tooltip 显示对标基准与 `回算`;
- `CoalPriceDialog`: 加载中 / 取不到 / 不足两天三种状态; 改过化验值时显示口径提示;
- 煤池卡片: 有快照的煤点估价行打开弹层且不触发卡片点击; 用户新增的煤不可点。

同步: `npm run check:consistency` 通过; `CLAUDE.md` 架构段 (`coal_prices.rs` 那句) 补上补算与历史接口。

## 部署后验证

- 启动日志出现补算汇总 (补了多少天、多少行、跳过几天);
- `doudou api GET '/api/coal-prices/history?name=临北'`: 从 2025-10-27 起每天一点, 10-29 起为 `live`;
- 10-29、10-30 两天的行与补算前逐字一致 (没被覆盖);
- 线上打开煤池点临北, 看曲线、虚实分段、10-09/10-15 附近的基准变更标记。

## 不做 (YAGNI)

- 改了算法或煤库数据后重算历史 (用户决定存下就不改);
- 一天内每小时的估价;
- 与真实成交价的回测对比 (等有成交价);
- 用户改过化验值的煤按用户值回算历史;
- MCP 工具与 `doudou` 子命令带历史。
