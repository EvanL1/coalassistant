# 每日估价快照与煤池卡片实施计划

> **For agentic workers:** 使用 superpowers:executing-plans 在本会话实施；TDD；最后独立审查工作区改动。用户已批准设计并要求直接实现，不再请求设计批准。

**Goal:** 共享煤库每小时整份重算当天快照，登录用户在煤池卡片看到估价与涨跌。

**Architecture:** 复用现有合并煤库、基准与 JSON 估价入口。纯函数与数据库 IO 分开；用户修改值只现算，不写全局快照。

**Tech Stack:** Rust / Axum / SQLx / PostgreSQL；React / TypeScript / Vitest。

**Spec:** `docs/superpowers/specs/2026-10-28-coal-price-daily-design.md`

## 全局约束

- 在 `feat/coal-price-daily` 当前工作区实施，不 commit、不 push、不动 `research/`。
- 不改核心与 master 数据；Rust 测试只用 fixture，不连接真实 PostgreSQL。
- 保留已批准的估价、煤种范围、日期与降级口径；注释和界面文案中文。

## 审查重点

- 读库或估价失败不可覆盖旧快照；事务保证删写原子性。
- 北京时间跨日不能把旧一轮计算写进已定格的昨天。
- 缺字段或非法系数表不能写入不完整快照。
- 编辑/重挂载后的异步旧结果不能覆盖新化验值的估价。
- 停用煤照常估价；无效覆盖值不送现算；无前日不编造涨跌。

## 三步执行

- [x] **1. 服务端快照与接口。** 新增 `coal_prices.rs` 与 `0011_coal_price_daily.sql`，修改 `lib.rs` / `benchmarks.rs` 以复用仅依赖 `Option<&PgPool>` 的读函数。先写日期边界、fixture 全量估价、基准价优先级、非法系数、compose 与 401/503 的失败测试，再实现 `beijing_day` / `snapshot_rows` / `compose`、事务 UNNEST、小时任务与路由。公开 DTO 为 `CoalPriceEntry` / `CoalPriceSnapshot`，字段严格对应设计。定向 `cargo test --locked coal_prices` 已通过 9 项。
- [x] **2. 前端数据与卡片。** 在 `types.ts` 添加 DTO；新增 `coalPrices.ts`、`CoalPriceLine.tsx` 及测试；修改 `CoalPoolScreen.tsx`。先测四种卡价分支、修改值/缺快照/无效值、请求失败、各显示文案和编辑后的请求竞争，再实现批量现算与安全状态更新。新 DTO 登记到一致性脚本。新增 33 项测试已通过。
- [x] **3. 校验与交付。** 更新 `CLAUDE.md` 架构说明，独立审查完整改动并处理重要问题；执行用户指定的服务端 fmt/clippy/test 与前端 test/build/consistency/data，记录每条结尾输出；检查 diff、无核心/master/research 改动，全部留在工作区。

## 实施记录

- 读库帮助函数须保留交互接口的降级能力，但把失败信息返回给快照任务，禁止用降级数据覆盖旧快照。
- 服务端 release 启用 unwind；现有 abort 会令设计要求的子任务 panic 隔离失效。
- 请求顺序：煤库与快照并行读取，快照到齐后把修改煤、用户煤及缺快照的煤合并为一次 POST；确定缺项依赖快照响应，因此不提前发不完整的现算批次。
- 基线：服务端库测试 93 通过 / 1 忽略；CLI 8 通过 / 1 因沙箱禁止监听本地端口失败。前端默认 Node Web Storage 使 42 测试失败，使用 `NODE_OPTIONS=--no-experimental-webstorage` 后 291/291 通过，无需改业务或测试配置。
- 第一轮服务端 TDD：8 个失败转 8 个通过；前端数据层与估价行：19 个失败转 25 个通过（降级分支初始桩已满足）。
- 独立审查：3 项 Important，均接受修复。缺 adjusts 要拒绝写快照；停用/隐藏不能遮蔽无效覆盖；同名冲突煤不可拖垮其余煤的批量现算。逐项补 RED 后修复，不改变核心算法。
- 审查范围裁定：真实 PostgreSQL 未运行（用户要求无真实库测试）；CCP 降级与经济模型沿用现状；空共享库没有当前管理入口可产生，允许空列表按整份替换语义处理。风险分别为 SQL 运行时行为待集成环境验证、既有来源/模型误差、未来若允许清空煤库须重新审视空快照语义。
- 三项审查修复：Rust 缺系数回归 1 项 RED → GREEN；前端无效状态/重名回归 3 项 RED → GREEN，连同原解析器测试 48/48 通过。
- 最终验收：`cargo fmt`、release Clippy 通过；完整 `cargo test --locked` 中库测试 102 通过 / 1 忽略，CLI 8 通过 / 1 因沙箱禁止端口监听失败（`src/bin/doudou.rs:599`, `Operation not permitted`，基线同样失败）。`NODE_OPTIONS=--no-experimental-webstorage npm test` 31 文件 / 324 测试通过；build、check:consistency、check:data 通过。日志 `/tmp/coal-price-validation/`。
- 第 3 步验收未全绿：旧 CLI 端口测试受沙箱限制；浏览器工具拒绝 Microsoft Edge 访问（未获权限），真实浏览器操作未完成。所有代码已完成并留工作区；未连接真实 PostgreSQL、未提交或推送。

## 交付前审查 (Claude, 2026-10-29)

- 修正: 快照里没有的煤 (含还没生成快照、接口 503) 也走现算, 原先一律标「按修改值估」—— 首次部署或本地无库时
  每张卡都会误标. `CardPrice.live` 加 `modified`, 只有用户改过化验值或自己新增的煤才标; 先改测试 (5 项 RED) 再修.
  原屏幕测试把误标写成了期望, 属测试本身错误, 一并改正.
- 整理: `beijing_day` 复用 `mcci::beijing()`; INSERT 手工排版 (原始字符串超宽, rustfmt 不处理该语句);
  新注释与日志改回仓库的半角标点.
- 本机 (沙箱外) 全量复验: fmt / release clippy 通过; `cargo test --locked` 库 102 通过 / 1 忽略, CLI 9/9 通过;
  Node 22 下 `npm test` 31 文件 / 325 测试通过 (不需要 `NODE_OPTIONS`); build、check:consistency、check:data 通过.
- 真实 PostgreSQL (docker postgres:17-alpine) 联调: 迁移、启动即算 (112 种全写入, 112 种都对上基准, 6 种带缺项)、
  `/api/coal-prices` 401/200; 造前一日数据后 prev_day 取最近一天、涨跌正确; 重启触发重算, 当天整份替换不重复.
- 浏览器 (Chrome) 看过煤池卡片: `估价 2,204 元/吨 ▲10`, 副标题 `估价 10-29`.
