# 询价精算 + 行情条常驻 设计

日期: 2026-10-30 · 状态: 用户已批准 (方案 A)

## 背景

真实配煤流程 (用户与朋友讨论得出): 凭感觉与记忆粗筛几种煤 → 询价 → 精确配比; 很多时候直接沿用原配方。
产品现在只有"全煤池 + 估价 → 最优配方", 缺后两步。10/27 定的"报价不进求解"今天由用户改为:

> 询价后肯定要把价格再录入, 这里有两个功能, 1、现场精算配方, 2、对比估算价格与真实价格的差别, 优化估算算法。

先估价、后询价的顺序不变: 估价用于粗筛与谈价参考, 询来的价用于当场精算, 并存档做估价的校准标签。
当初删报价的理由 (旧价把配方锁在错的价位上) 仍成立, 所以询价**当场填、当场有效**, 不进煤池、不跨次沿用。

## 范围

做:
1. 今日页「询价精算」: 选煤 → 填出厂价与运费 (可当场改这批指标) → 只用填了价的煤精算 → 保存到历史。
2. 询价存档 `coal_quotes`: 每条带服务端按"这批指标 + 当天基准"重算的同口径估价。
3. 后台「询价对比」: 明细 + 按煤种汇总 (报价 / 估价 中位数)。
4. 行情条 (炼焦煤价格指数) 在今日页与煤池页顶部常驻, 页面滚动时不移走。

不做 (以后单独一期):
- 用询价数据自动调升贴水系数 —— 现在 0 条询价, 攒到几十条再做。本期的存档就是为它准备的。
- 手机 Agent (MCP) 的精算、原配方体检、`robustness` 的询价清单。

## 已定口径 (用户逐条确认)

| 问题 | 决定 |
|---|---|
| 没询价的煤 | 不参与精算。只用填了出厂价的煤; 少于 2 种不能精算, 提示先去问价。 |
| 价格口径 | 出厂价 + 运费分开填。精算按到厂价 = 出厂 + 运费; 对比估价只用出厂价 (估价是出厂含税口径)。运费不填记 0。 |
| 时效 | 当场有效。一次精算会话填的价只用于这一次; 下次重新填, 不自动带入上次的价。 |
| 这批指标 | 默认用煤池指标, 可当场改; 改动只用于这一次, 随报价存档, 不写回煤池。 |

## 用户流程

```
今日页 (估价配方已算好) ──「询价精算」──▶ 精算页 (仍在今日标签内, 顶部「← 返回今日」)

选煤: 默认列出今日估价配方里的煤, 可从煤池 (启用且数据有效的煤) 再加, 可去掉
  每行: 煤名 · 今日估价 · [出厂价] [运费] · ▸改这批指标
        没进今日配方的煤多一行灰字: 到厂价低于 X 元才会配进去 (X = 今日结果的 blend_value)
        报价偏离今日估价 > 30%: 行内提示"比今日估价高/低 N%, 请核对" (只提示, 不拦)
        上次询价 (若有) 只作输入框占位提示, 不作为值
  ↓ 填了出厂价的煤 ≥ 2 种, 「精算」可点
精算: 配方 + 成本 + 指标 (与今日页同样的卡片) + 每种煤「报价比估价 高/低 X 元」
  同时把这批询价存档 (失败只提示"询价未存档", 不影响结果)
  ↓
[保存到历史] → 历史页照常回填实测; 带报价的记录显示「询价」标记
```

- 合同、采购吨数、一铲最小量、采购保证值扣款条款与今日页完全一致。
- 离开今日标签 (或点返回) 即结束本次会话; 精算页状态不持久化。
- 已进今日配方的煤不显示"低于 X 才会配进去": 线性规划的对偶对已入选的煤给出的就是它的现价,
  显示出来没有信息量, 反而误导。

## 前端

**新文件**
- `doudou_blend/src/screens/QuoteScreen.tsx`: 精算页 (选煤、填价、改指标、结果、保存)。由 `TodayScreen`
  持有 `quoting` 状态、原地替换今日页内容来渲染, 今日的估价结果 (snapshot) 作为 props 传入
  (默认煤、今日估价、`blend_value` 都从它取)。
- `doudou_blend/src/quotes.ts`: `archiveQuotes(batchId, items)` → `POST /api/quotes`; 校验返回形状
  (同 `coalPrices.ts` 的写法)。
- `doudou_blend/src/domain/blendRequest.ts`: 从 `TodayScreen.runSolve` 抽出"合同 + 吨数 + 一铲 + 价格因子
  (系数表 + 基准)"的请求拼装, 今日页与精算页共用。今日页行为必须不变 (现有测试原样通过)。

**精算请求** (核心不改):
```jsonc
{
  "coals": [ /* 只含填了价的煤: 指标 = 煤池值 ∪ 本次改动, fob = 出厂价, frt = 运费, purchase_terms 同今日页 */ ],
  "specs": [...], "total_quantity": ..., "shovel_tons": ..., "truncate_decimal": true,
  "price_factor": { /* 同今日页的系数表与基准 */ "extra_coals": [], "estimate_unpriced": false }
}
```
核心 `price_factor::attach` 对 `coals` 做估价对比, 结果里 `coal_values[].fob/fair/gap` 就是"报价 vs 估价",
不需要新字段。

**会话与存档**: 精算页挂载时 `batchId = crypto.randomUUID()`。每次点「精算」并行做两件事: 求解; 存档本批
(条目 = 当前填了价的煤)。同一批多次精算以最后一次为准 (谈价是 2200 → 2180 → 2150 的过程, 只有最后的价
是有意义的标签)。

**结果展示**: 复用 `CostCard`; 今日页里内联的配方卡、指标卡抽成组件共用, 不复制。新增"报价对比"表:
煤 · 用量 · 出厂报价 · 估价 · 差额 (高/低)。

**历史标记**: 历史 DTO 加派生字段 `quoted` (结果里任一 `coal_values[].fob` 非空), 历史页显示「询价」小标。
服务端历史表不改。

**行情条常驻**
- `App.tsx`: `IndexTicker` 在今日与煤池两个标签都渲染 (仍在页面最上方)。
- `App.css`: `.index-ticker { position: sticky; top: 0; z-index: 10; background: var(--c-bg); }`, 宽屏
  (≥720px) 的 background 改为与 body 一致的 `#eef1f5`。页面是 window 滚动 (`.app-content` 没有 overflow),
  sticky 直接生效。走势弹层 z-index 1000、宽屏侧栏 20, 都不受影响。
- 精算页在今日标签内, 所以谈价时行情条也在顶上。

## 服务端

**迁移 `0013_coal_quotes.sql`**
```sql
-- 询价存档: 用户当场询来的出厂价与运费, 附服务端按这批指标 + 当天基准重算的同口径估价.
-- 只用于对比与以后校准估价, 不进煤池、不跨次沿用. 同一批 (batch_id) 多次精算以最后一次为准.
CREATE TABLE IF NOT EXISTS coal_quotes (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    quoted_on DATE NOT NULL,             -- 北京时间
    name TEXT NOT NULL,
    coal_type TEXT,
    form TEXT,
    region TEXT,
    fob DOUBLE PRECISION NOT NULL CHECK (fob > 0),
    frt DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (frt >= 0),
    props JSONB NOT NULL,                -- 这批实际用的指标 (全量)
    changed JSONB NOT NULL DEFAULT '[]', -- 当场改过的指标名
    estimate DOUBLE PRECISION,           -- 同口径估价 (出厂); 估不出为 NULL, 询价照存
    points DOUBLE PRECISION,
    benchmark TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (username, batch_id, name)
);
CREATE INDEX IF NOT EXISTS coal_quotes_day ON coal_quotes (quoted_on);
```

**新模块 `blend_kit_server/src/quotes.rs`**

`POST /api/quotes` (登录会话):
- 请求 `{ "batch_id": "...", "items": [{ "name", "fob", "frt"?, "coal_type"?, "form"?, "region"?, "props", "changed"? }] }`。
- 检查顺序: 401 → 400 (请求体) → 503 (没有数据库)。
- 校验: `batch_id` 1–64 位 `[A-Za-z0-9-]`; `items` 1–50 条且煤名不重复、非空、≤100 字;
  `fob` 有限且 0 < fob < 100000; `frt` 有限且 0 ≤ frt < 10000; `changed` 只能是 `INDICATORS` 里的名字;
  指标交给核心估价校验 (估价返回错误 → 400 并带核心的错误信息)。
- 估价: 拼 `{"coals": items(带 fob)}` → `benchmarks::fill_price_request` (线上当前系数表与基准) →
  `blend_kit::estimate_json`, 取每条的 `fair / points / benchmark`。不信任浏览器传来的估价。
- 写库: 一个事务里 `DELETE ... WHERE username = $1 AND batch_id = $2`, 再插入本批 (按批覆盖)。
- 返回 `{ "ok": true, "items": [{ "name", "estimate", "gap", "ratio", "benchmark" }] }`。

`GET /api/admin/quotes?limit=` (管理员):
- 明细: 最新在前, 默认 200 条, 上限 1000; 字段同表 + `gap = fob − estimate`、`ratio = fob / estimate`。
- 汇总 `by_type`: 按 `coal_type` (空记"未标煤种") 分组, 只算有估价的行; 与 `admin::calibration` 同口径,
  **不算管理员账号自己的询价** (管理员试用时的假价不进汇总; 明细仍全部列出, 带用户列)。
  每组 `count`、`median_ratio`、`median_gap`。
- 路由加在 `lib.rs`; 新的共享结构登记到 `scripts/check_repository_consistency.mjs`。

`doudou` 命令行不加子命令, 用 `doudou --profile admin api GET /api/admin/quotes` 即可。

**后台页**: `AdminScreen` 加「询价对比」卡片: 按煤种汇总表 + 明细表。

## 错误处理

- 精算求解失败或不可行: 与今日页同样展示原因; 存档照常 (询价本身是有效数据)。
- 存档失败 (网络 / 503 / 400): 结果照常显示, 页面一行提示"询价未存档: 原因"; 下次精算会再试 (同一批覆盖)。
- 估价算不出 (无基准等): 该条 `estimate` 存 NULL, 对比表显示"—"。

## 测试

- 服务端 (fixture, 不断言 master 内容): 请求校验各分支; 估价由服务端算 (传入伪造估价字段被忽略);
  按批覆盖 (同批第二次少一条 → 库里也少一条); 401/400/503 顺序; admin 汇总排除管理员、中位数正确、
  估价为 NULL 的行不进汇总。真实 PostgreSQL 联调 (docker) 跑一遍迁移与写读。
- 前端: `blendRequest` 抽取后今日页现有测试原样通过; 精算页: 默认带出今日配方的煤、少于 2 种不能精算、
  只把填了价的煤放进 `coals` 且 `extra_coals` 为空、改指标只进本次请求不改煤池、偏离 30% 提示、
  "低于 X 才会配进去"只对未入选的煤显示、每次精算都存档且同一 `batch_id`; 历史「询价」标记;
  行情条在煤池标签渲染。指数弹层现有测试一行不改。
- 本机浏览器: 精算全流程; 今日页与煤池页滚动时行情条停在顶上, 弹层盖在它上面。

## 文档

`CLAUDE.md`: 写明"估价之后的询价精算"与 `coal_quotes`, 并改掉"Real quotes … never enter the solve"
那段 —— 现在是: 求解默认全按估价; 只有询价精算按当场询价, 且不进煤池。

## 风险

- 用户在精算页填的价进入公共汇总前没有人工审核: 30% 提示 + 汇总取中位数 + 排除管理员账号, 先这样;
  校准那一期再加剔除。
- `TodayScreen.tsx` 已近 900 行, 抽 `blendRequest` 与配方/指标卡组件是为精算页复用, 不做别的重构。
