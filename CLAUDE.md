# CLAUDE.md

This file provides guidance to coding agents working in this repository. `AGENTS.md` is a symlink to it, so
Claude Code, droid, Codex and others all read the same text — edit this file only.

## What this is

豆哥配煤 (Doudou Blend) — a coking-coal blend optimizer. It takes a pool of coals (each with 8 quality
indicators + an FOB/FRT price split) plus quality specs, and solves a linear program for the cheapest blend
ratio that satisfies every constraint. Ships as a browser web app: a React frontend against a Rust HTTP
server.

## Repository layout (three independent Rust crates — NOT a cargo workspace)

| Path | Crate / package | Role |
|------|-----------------|------|
| `blend_kit_rs/` | `blend_kit` | Pure LP core. Clarabel solver, embedded master data. Zero web/WASM deps. |
| `blend_kit_server/` | `blend_kit_server` | Axum HTTP server. Serves `/api/*`, PostgreSQL persistence, auth. Depends on `blend_kit` by path. |
| `blend_kit_wasm/` | `blend_kit_wasm` | Thin `wasm-bindgen` wrapper. Depends on `blend_kit` by path. **Currently unused by the app** — the frontend talks to the server, not WASM. Kept as a standalone build target. |
| `doudou_blend/src/` | (npm) `doudou_blend` | React 19 + TypeScript + Vite frontend. |

Each crate has its own `Cargo.toml` and `Cargo.lock`; build them from inside their own directory.

`mockup/` is the original HTML visual mockup of the six screens. A local untracked `mines.db` (if present) is a
leftover of the removed Tauri/SQLite backend; nothing reads it.

## Architecture — the parts that span files

**One algorithm, two callers.** The entire core is reached through a single string-in/string-out function,
`blend_kit::solve_json(&str) -> String` (`blend_kit_rs/src/lib.rs`). It is invoked:
- directly in tests/examples and by the server (`POST /api/solve`),
- via `wasm-bindgen` as `solveJson` (`blend_kit_wasm/src/lib.rs`) — built, but not wired into the app.

Keeping every boundary at "JSON string in, JSON string out" is what keeps the core independent of transport.
Don't add typed cross-boundary APIs; extend the JSON request/result shapes instead.

**Single backend seam.** `doudou_blend/src/backend.ts` exposes one async interface (`solveJson` /
`getMasterJson`, plus blend history: `saveHistory` / `countHistory` / `listHistory` /
`setMeasuredQuality` / `clearHistory`) over same-origin `fetch('/api/...')`, so screen code never touches
transport details. History persists to PostgreSQL through the server and normalizes to a uniform
`HistoryRecord` DTO (the TS adapter derives recipe + the 6 mixed indicators from the stored result; the Rust
side stays dumb, storing an opaque blob + the `csr_measured` / `*_measured` columns — the backfill covers the
full assay sheet CSR + S/A/V/G/Y/M, feeding both the CSR regression and the future G-correction fit, plus
optional coke CRI/M40/M10 and coking conditions — bulk density / coking hours / flue temp — to separate coal
effects from oven effects).

**Data flow inside the core** (`blend_kit_rs/src/`):
`model.rs` (Coal/Spec/BlendRequest/BlendResult types, 8-indicator constant) →
`optimizer.rs` (builds the Clarabel LP: minimize Σ cif·xᵢ s.t. Σxᵢ=1 and per-spec weighted ≤/≥ bounds; also
computes slack + `binding` flags) →
result is post-processed into three business views: cost breakdown, physical orders, indicator check.
`seed.rs` loads the embedded master DB + status state machine; `predict.rs` is optional CSR regression.
`rank_interaction.rs` is the opt-in blend-incompatibility term (`BlendRequest.rank_interaction`: CSR − k·Var(Ro),
linearized per round so the LP stays linear); absent or k = 0 must leave results byte-identical.
`pricing.rs` is the standalone **estimation module** (估价模块): quality index (1000-point premiums/discounts from
`price_factor.adjusts`) + pricing against **benchmarks** (`price_factor.benchmarks`: dated market quotes per coal type) —
each coal is priced by **indicator distance, not by coal type** (`price_factor.matching`): distance = premium differences
(points) + unpriced indicators such as V (`matching.weights`, points/unit; coal types are defined by V/G/Y, so once V is in
the distance the `coal_type` label is redundant) + a place penalty for non-local benchmarks (soft preference — forcing
a sparse local benchmark mis-prices by hundreds; place is three fields `province` / `city` / `county` holding official
administrative-division names, compared 省 → 市: other province `region_penalty`, same province other city
`city_penalty`, same city or a province-wide benchmark 0; county is recorded but not compared; the free-text `region`
is legacy, used only when a coal has no `province`. Entry UIs pick from `doudou_blend/src/data/divisions.json` via
`PlaceSelect`, and `check:data` validates master against the same file); all benchmarks compete, shares ∝ exp(−d/`bandwidth`), and
`price = Σ share × benchmark × Q(coal)/Q(benchmark)` (`CoalValue.benchmark_mix` lists the shares). The form layer (`Coal.form`:
精煤/筛选精煤/浮选精煤…; applied only if such a benchmark exists) stays hard — form is not encoded in V/G/Y. Only when the
coal lacks a `matching.weights` indicator (or `matching` is absent) is the hard coal-type layer applied first (blending, if
configured, then happens within it; without `bandwidth` only the single nearest benchmark is used). The coefficients
come from a leave-one-out backtest (see master `matching_note`). Without benchmarks it falls back to `market_price`. Own JSON entry `estimate_json` (server `POST /api/price`, CLI `blend price`);
`price_factor.rs` is only the solver glue (unpriced coals enter the LP at the estimate; `coal_values` attached after).
Benchmarks live in three layers merged by the server (`blend_kit_server/src/benchmarks.rs`): master defaults + the
`price_benchmarks` table (admin-entered, latest date per name wins) + the **Mysteel coking-coal index** (MCCI,
`blend_kit_server/src/mcci.rs`): the server pulls 7 fixed-spec series daily after 17:30 Beijing into `mcci_daily`
(signed request copied from the public index page; `MCCI_FETCH=off` disables), and each series' latest value becomes a
benchmark whose props are the standard product in master `price_factor.index_series`. The composite index (`LJM_ZH`)
has no fixed spec, so it sits in `price_factor.display_series`: fetched and shown first in the ticker, never a
benchmark or drift source. Older quotes are drifted into
`current_price` by the MCCI series nearest by the same distance (picked through `estimate_json`; a quote without V only
considers same-coal-type series) when it can, else by CCP. CCP is index points
(2017-01 = 1000) — never use it as a price. Master-only `price_factor` keys (`*_note`, `index_series`) are stripped before
reaching the core (`MASTER_ONLY_KEYS`, mirrored in `doudou_blend/src/benchmarks.ts`).
`blend_kit_server/src/coal_prices.rs` 在服务启动后每小时重算当天 (北京时间) 的共享煤库估价快照, 并在启动时与每过零点
按"当天能拿到的指数与报价" (`benchmarks::as_of`) 补算今天以前缺的 (日期, 煤) —— 存下就不改, 只补空 (`source` 区分
live / backfill); `GET /api/coal-prices` 返回最新日与前一日价格, `GET /api/coal-prices/history?name=` 返回单煤全部历史;
煤池卡片据此显示估价与涨跌, 点估价行看走势; 用户修改或新增的煤按当前值现算.
`blend_kit_server/src/forecast.rs` 是**一周后的价格预测** (只作参考展示, 不进求解): 每条指数序列未来 5 个交易日的
涨幅, 对自己过去 1/5/20 日涨幅与综合指数过去 1/5 日涨幅做线性回归, 用全部历史现拟合 —— 所以 `mcci_daily` 从指数
最早一期 (2018-04) 回填, 估价仍只读近 400 天; 历史不足约两年、或指数停更一周以上就不给预测. 每小时随估价快照算一次:
指数的预测存 `mcci_forecast` (留作日后对照实际值), `GET /api/mcci` 每条序列带 `forecast`; 煤的预测价是把预测值当作
"一周后那一期指数" 重新定基准、把共享煤库重估一遍 (`coal_prices::forecast_prices`), 存 `coal_price_daily.forecast_price`
/ `forecast_day`, 补算的历史不带. 界面在煤池卡片、煤价走势与指数走势弹层里显示 (`priceForecast.ts`). 模型取舍与回测见
`research/price_forecast/` (20 个交易日以上没有可预测性, 不要加更长的期限).
`blend_kit_server/src/quotes.rs` 接收现场询价, 按这批指标与当天基准重算出厂估价, 在一个事务里按批覆盖存档,
并提供最近询价占位提示、后台明细/按煤种汇总/删除.
`robustness.rs` is the opt-in price-robustness analysis (`BlendRequest.price_robustness`): prices are mostly
estimates and the LP optimum is flat, so it re-solves with perturbed prices to report per-coal flip thresholds (how
many 元 before ≥10% of tonnage moves, vs. that coal's price uncertainty from `price_factor.uncertainty`, keyed by
`Coal.coal_type`) and a "robust recipe" (risk premium λ·uncertainty, within a cost tolerance). Absent must leave
results byte-identical.
`shovel.rs` is the **一铲最小量** (`BlendRequest.shovel_tons`, with `total_quantity`): loaders blend by the shovel, so
each coal is either unused or ≥ 1 shovel, in whole shovels (one shovel = 1/N, N = round(total / shovel)).
Branch-and-bound over "drop it / floor it at one shovel" (floors go into the LP as `BlendRequest.min_ratios` rows),
then largest-remainder rounding to whole shovels (`OrderItem.shovels`) evaluated through the fixed-ratio path plus the
exact petrography review (`optimizer::review_petrography`, shared with the LP path), with a one-shovel-swap repair if
rounding breaks a spec. 合同按「保留 1 位小数、截断」判定 (用户确认的真实规则), LP 把上限用到下一格减 ε, 配方常顶在
合同界上、一点余地都没有, 挪一铲也救不回; 这时给被凑整顶破的那几项加安全余量 (`Spec::margin`, 按判定步长的
`HEADROOM_STEPS` 逐级加大) 重新分支定界再凑整, 凑整后的验算仍按原合同, 成功时置 `BlendResult.shovel_extra_cost`
(比不凑整多花的元/吨, 只在有值时序列化); 都不行才退回未凑整的配方并提示。Blend values come from the root LP; price-robustness re-solves also go through the shovel
wrapper. The shovel size is a per-user setting (我的 page, `web_user_state.shovel_tons`, NULL = default 3 t; saved with
COALESCE so pre-deploy clients can't reset it). Absent = byte-identical.

**求解失败分两种, 不要混报.** 求解器证实不可行, 或逐项诊断找到了真凶, 报"约束冲突, LP 不可行" (原文被界面
和历史记录照存, 不要改字)。求解器迭代用尽/数值错误而诊断也证实不了时, 如实说没算出可靠结论, 并置
`BlendResult.undetermined` (只在为真时序列化) —— 界面与 MCP 据此不引导用户去放宽合同。解后复核差一丝没过时
只收紧没过的行重解。

**输出的配方必须自己经得起验算.** `recipe` 与订单默认只含占比 > 1e-5 的煤 (再小是求解残差)。但合同卡在判定
边缘时, 解可能靠一种占比百万分之几的煤才达标; 截掉它, 照着输出的配方回放 (`fixed_ratios`) 就不达标。所以
`optimizer::output_grain` 在"截掉零头后的配方按合同判定不达标"时, 从大到小把零头煤放回、取刚好达标的门槛。
留下来的零头煤在按整铲配时会被分支定界看见: 要么去掉, 要么给足一铲。

**服务端的计算闸门** (`blend_kit_server/src/limits.rs`): 求解与估价共用一个按核数的并发上限加超时;
登录有全站并发上限, 按来源 IP 锁定默认关 (`LOGIN_IP_LOCKOUT`, 见 `docs/railway-deployment.md`)。

**Frontend screens** (`doudou_blend/src/screens/`): 今日 `TodayScreen` (solve + cost/recipe/8-indicator view,
real purchase-qty input, export orders, save-to-history, input-summary panel), 煤池 `CoalPoolScreen`
(enable/hide + assay overrides; **no price entry**), 合同 `ContractScreen` (quality specs), 历史 `HistoryScreen` (saved
blends + measured-CSR backfill), 我的 `MeScreen`. Tabs mount/unmount on switch (`App.tsx`
`{tab === ... && <Screen/>}`), so each screen reloads its data on entry — relevant for async backend reads.
The whole app sits behind a login gate: `App.tsx` renders `LoginScreen` until `tryLogin` (`auth.ts`) succeeds
— credentials are checked server-side and the session is an HttpOnly cookie, so no password ships in the
frontend bundle. Screens read master data through `master_loader.ts`'s `loadMaster()` module cache; call
`invalidateMaster()` after anything that changes master data.

**Master vs. user data split.** `coal_master.json` (embedded in `blend_kit_rs/data/`, served read-only via
`GET /api/master`) is the canonical coal set and is never mutated. User edits (enable/hide, assay overrides) persist separately:
`localStorage` acts as a synchronous cache (`doudou_blend/src/storage.ts`) and `cloudStorage.ts` mirrors it to
PostgreSQL through `GET/PUT /api/storage` after login.

**云端同步的规则.** 线上所有登录者共用一个 Web 账号、同一行数据, 写回是整份快照、服务端没有版本号。
所以 `cloudStorage.ts` 在本机记两样东西: "有改动没同步"的标记, 和上次确认与云端一致时的基线
(`canonicalUserSnapshot` 的稳定序列化)。初始化时与每次写回前都先拉云端和基线比: 云端还停在基线上才把本机
推上去; 被别人改过就不推, 由 `SyncBanner` 让用户选用云端还是用本机, 选之前不往云端写任何东西。
拉与写之间仍有一次往返的窗口, 要彻底堵住得给服务端加版本号。管理员会话不认领也不清空本机的用户数据
(`localAccount.ts`)。所有接口经 `http.ts` 的 `apiFetch`, 401 统一回登录页。

**估价与询价精算.** 求解默认全按估价: `domain/resolvedCoal.ts` 的 `toEstimateCoal` 与 `today.rs` 都不读
master 或偏好里的价格键, 启用煤放进 `price_factor.extra_coals`, `coals: []`, 打开 `estimate_unpriced`.
唯一例外是今日页「询价精算」: 用户当场填几种煤的出厂价与运费, 只用填了出厂价的煤求解 (至少两种),
按到厂价 = 出厂价 + 运费计算. 当场改的指标只进本次请求与存档, 询价不进煤池、不写本地或云端偏好、不跨次沿用;
下次进入时输入为空, 最近询价只作 placeholder. 每次精算按批存档到 `coal_quotes`, 同一会话后一次覆盖前一次.
存档估价由 `blend_kit_server/src/quotes.rs` 按这批指标与当天基准重算, 运费不进估价; 没匹配基准记 NULL,
不拿本批报价反推的价格做对比. 存档失败不影响精算结果, 精算不可行仍存档. 后台「询价对比」按煤种汇总
报价/估价中位数与差额中位数, 可删除填错的记录, 供以后校准估价系数. 精算只给客户算配方, 不提供保存到历史.
旧偏好价格键 (`fob_override`/`frt_override`/`fob_history`/`is_price_anchor` 等) 仍在读取时剔除;
`check:data` 仍禁止 master 带 `fob`/`frt`. 行情条在今日 (含精算页) 与煤池页顶部常驻.

**Blend history & the CSR data loop.** Saved blends persist to PostgreSQL via `/api/history`, keeping the full
`BlendResult` JSON. The History screen lets the user backfill the lab-measured coke CSR onto a past blend —
pairing that blend's mixed indicators (regression **X**, recovered from the stored result) with the measured
CSR (regression **y**) into a `CsrObservation`. This is the data-collection half of the loop; **the
observations are not yet fed into `solve_json`** (deferred until enough accumulate), at which point
`predict.rs`'s gated regression activates.

## Build & test commands

**Core algorithm** (most common loop — fast, no system deps):
```bash
cd blend_kit_rs
cargo test --release                          # all tests (lib.rs + per-module #[cfg(test)] + tests/, incl. 性质测试)
cargo test --release test_basic_solve         # a single test by name
cargo run --release --example demo
cargo run --release --bin blend -- eval < request.json   # 命令行 (给 agent/脚本): solve 求最优 / eval 按 fixed_ratios 验算 / master 输出内置数据
cargo clippy --release --all-targets -- -D warnings   # lint 的放行与禁止写在 Cargo.toml [lints.clippy]
cargo fmt
PROPTEST_CASES=200000 cargo test --release --test properties   # 性质测试深跑 (默认每条几千例, 不到 1 秒)
cargo test --release --test properties -- --ignored            # 已知的极薄可行域反例 (如实报"没算出结论")
```

**Web server** (`cd blend_kit_server`):
```bash
cargo test --locked
cargo clippy --all-targets -- -D warnings      # 生产代码禁 unwrap / expect / panic, 见 Cargo.toml [lints.clippy]
DATABASE_URL=... cargo test --locked --lib -- --ignored --skip live --test-threads=1   # 库测试 (支架 src/test_db.rs)
cargo run                                      # needs DATABASE_URL; 没配齐 AUTH_* 时用内置默认口令且只监听 127.0.0.1
cargo run --release --bin doudou -- --profile admin login <账号>   # 线上命令行, 见下
```

**线上命令行 `doudou`** (`blend_kit_server/src/bin/doudou.rs`): 登录线上服务, 走 `/api/*`, 与网页同一套数据
(煤库覆盖值、后台设的 k、历史)。`--profile` 切换账号 (普通用户 / 管理员), 档案存 `~/.config/doudou/profiles.json`
(600, 只存会话 cookie)。密码读 `DOUDOU_PASSWORD` 或终端输入。命令: `login/logout/whoami/profiles`、`solve/eval`、
`master`、`history [measured <id>]`、`admin settings [--k|--off]|calibration|csr-model|keys|overrides`、`api <METHOD> <路径>`。
`blend` 只在本机用内置 master 离线算; 要和用户看到的一致, 用 `doudou`。

**MCP 入口 `/mcp`** (手机 Agent, 如 Kimi; 配煤咨询按 打分 → 计算 → 询价 → 再计算 四步走): 无状态 JSON-RPC, 六个工具
`today_blend` / `check_ratios` / `quote_blend` / `find_coal` / `get_contract` / `market`。鉴权用后台生成的 **mcp 用途**密钥
(`data_api_keys.scope`, 查询+求解+询价存档, 写不了煤库; data 用途的写煤库密钥进不来)。求解请求由 `src/today.rs` 按线上
保存的合同与煤池组装 —— 规则对照前端 `domain/resolvedCoal.ts`, 改了前端的煤池规则要同步改这里。每个客户的合同要求、
候选煤、报价按次传 (`specs` / `only` / `quotes`, 覆盖规则在 `src/today_overrides.rs`), 只改这一次的请求, 不写存储。
`quote_blend` 是 MCP 侧的询价精算 (只用报了价的煤, 按到厂价求解), 报价按批存档到 `coal_quotes`: `mcp.rs` 只算出要存的
那一批 (`quote_archive`, 批号 `mcp-<日期>-<煤名摘要>`, 同一天同一组煤覆盖), 落库在 `lib.rs` 回包算完之后走
`quotes::store` (与 `POST /api/quotes` 同一条路; `store` 带批号那一天, 成败返回给调用方): 只给回包是 result 且工具
没报错的调用存 (求解闸门拒绝的不存), 一个请求最多存最后一条算成的, 「已存档 / 没存成」由 `lib.rs` 补到回包文本结尾,
失败另记日志。`find_coal` 带每日估价快照的涨跌与一周后预测,
`market` 给指数现值、涨跌与预测 (`src/mcp_market.rs`; 预测只作参考, 不进求解)。`mcp.rs` 及其子模块保持纯函数:
不查库不联网, 新数据加到 `Context`, 在 `lib.rs` 的 `load_mcp_context` 里装。接入说明见 `docs/mcp-agent.md`。

**WASM package** (standalone — not needed for the app):
```bash
cd blend_kit_wasm
wasm-pack build --target web --out-dir pkg --release
node test_wasm_solve.mjs                       # smoke-test the built .wasm
```

**Frontend** (`cd doudou_blend`, needs Node 22+):
```bash
npm install
npm run dev          # Vite dev server; proxies /api to the local blend_kit_server
npm run build        # tsc && vite build → dist/
npm test             # vitest
npm run check:data   # master 数据自检
npm run check:rules  # 前端分层约定 (见下 "机器检查的约定")
```

## 机器检查的约定 (改代码前先看, 违反了 CI 会红)

下面这些以前只是口头约定, 现在各有一条检查守着; 报错信息里写了根因与改法。要改约定本身, 就连检查一起改。

- **求解与判定同一条线** (`blend_kit_rs/tests/properties.rs`, proptest): 预埋可行解的合同不许报冲突 (解不出时只能是如实的"没算出结论",
  且占比极小 —— 可行域薄到求解精度以下的反例单列在 `#[ignore]` 里);
  输出的配方回放后结论一致; 超出合同一格必判不达标; 整铲之和等于总铲数; 可选开关缺省时输出逐字节相同;
  估价对基准价齐次。执行界、判定一格、放行量只在 `quality.rs` 各有一个函数 (`executed_upper/lower`、`quantum`、
  `judging_allowance`), 不要在别处再写一遍公式。
- **输出逐字节确定**: 会序列化的映射用 `BTreeMap`, 不用 `HashMap` (`tests/deterministic_output.rs`)。
- **服务端鉴权在路由表里**: 新路由加进 `lib.rs` 的 `routes()` 并写明 `Access` (公开 / 会话 / 管理员 / 写煤库密钥 /
  MCP 密钥)。鉴权是按 `Access` 套在路由上的中间件, 在读请求体之前做完, 所以未登录一律 401, 不会先解析
  请求体; handler 里用 `Authed<…>` 提取器取身份。三条路由表测试守着: 不带凭据的一律 401; 每类 `Access`
  只放行自己那种凭据 (普通会话进不了管理接口); 路径与登记的类别不能矛盾 (`/api/admin/*` 必须是管理员,
  `/api/master/*` 必须是写煤库, `/mcp` 的 POST 必须是 MCP 密钥)。`AUTH_*` 设成空串按没设处理。
- **MCP 工具是枚举**: 新工具加进 `mcp.rs` 的 `tools!`; 是否过求解闸门、要不要装行情数据、会不会存档都是对它的
  穷尽 `match`, 漏了编不过。
- **纯函数模块**: `mcp.rs`、`mcp_market.rs`、`mcp_quote.rs`、`today.rs`、`today_overrides.rs` 里出现
  `sqlx` / `reqwest` / `tokio::` / `.await` 会被单元测试拦下; 查库联网放 `lib.rs` 的 `load_mcp_context`。
- **前端分层** (`scripts/check_frontend_rules.mjs`): 只有 `http.ts` / `auth.ts` 直接调 `fetch`, 数据模块用
  `apiJson` / `apiJsonOrNull`; 屏幕与组件不导入 `apiFetch` 之类; `localStorage` / `sessionStorage` 只在
  `storage.ts`、`cloudStorage.ts`、`penaltyStorage.ts`、`localAccount.ts`、`coal_index.ts`、`domain/quoteDraft.ts`;
  事件名只写在 `events.ts` (用 `emit` / `listen` / `useAppEvents`); 询价页只从存储模块导入 `get*` 与类型;
  旧价格键只在 `storage.ts` 剔除。
- **Rust 与 TypeScript 两边各写一份的常量** (`scripts/check_repository_consistency.mjs`): 除结构体字段名、枚举、
  指标顺序外, 还比对 `MASTER_ONLY_KEYS`、煤状态列表、报价上限、回填量程、一铲默认吨数、冲突原因原文等;
  新加一对这样的常量就在脚本里登记。
- **云端同步的判断是纯函数** (`cloudStorage.ts` 的 `decideInit` / `decidePersist` / `decideWrite`), 有穷举表测试
  (`cloudStorage.decisions.test.ts`); 改同步规则先改表。

## Critical gotchas

- **`blend_kit_rs/data/coal_master.json` is the only Master source.** `blend_kit::master_json()` serves the
  same embedded bytes to the server (`GET /api/master`) and the WASM wrapper. Do not add a frontend static copy.
- **Data changes must not require code changes.** Rust tests use fixtures and never assert master contents;
  data invariants live in `scripts/check_master_data.mjs` (`npm run check:data`). If a data update turns a
  Rust test red, fix the test's coupling, not the data.
- **`doudou_blend/src/types.ts` mirrors the Rust schema.** Change a shared Rust struct, serde field, enum, or
  indicator ordering together with its TypeScript definition. Run `npm run check:consistency` from
  `doudou_blend/`; CI compares shared fields, enum values, indicator order, versions, documentation, and license.
- **`blend_kit_wasm` disables `wasm-opt`** (bundled version is too old for Rust 1.82+ bulk-memory). Don't
  re-enable it. Size is already controlled via `opt-level="z"` + LTO.
- **`Direction::Upper` ignores `spec.min`; `Direction::Lower` ignores `spec.max`** by design — only `Range`
  uses both bounds. There are regression tests pinning this (`test_direction_upper_ignores_min`).
## CI / deploy

- `ci.yml` (push/PR) is the only workflow: consistency + master-data + frontend-rules checks, frontend
  test/build, `blend_kit` tests (incl. property tests) + example + doc-link check, `blend_kit_server` tests
  (incl. DB tests), `blend_kit_wasm` compile check, fmt and clippy.
- Production deploy is **Railway + Supabase** via the repo-root `Dockerfile` (Node stage builds React, Rust
  stage builds `blend_kit_server`, final image ships the server + static assets). See
  `docs/railway-deployment.md` for the required `AUTH_*` / `DATABASE_URL` env vars.
- Railway 服务设置由 `.railway/railway.ts` 声明 (根目录 `package.json` 只为装它的 SDK)。它是**声明式**的:
  没写的东西 apply 时会被删掉 —— 代码来源要写 `github(...)`, 环境变量要逐个声明, 机密用 `preserve()`。
  改完先 `railway config plan`, 确认 **0 to destroy** 再 `railway config apply`; 不要用
  `railway config migrate` 覆盖它 (生成的版本会丢掉仓库来源、变量和重启策略)。

## Agent 环境 (E2B)

根目录 `Dockerfile` 是唯一的镜像定义: Railway 部署用它, 最终镜像里同时带 `/usr/local/bin/blend`
(离线) 与 `/usr/local/bin/doudou` (线上) 命令行, 所以这个镜像本身就是 agent 环境。E2B 不支持多阶段 Dockerfile, 模板要从构建好的镜像创建
(fromImage), 模板定义不放在本仓库。master 编进二进制, 改了 `coal_master.json` 要重建镜像。

## Conventions

- Commit messages: `<type>(<scope>): <desc>` in Chinese, e.g. `feat(coal-pool): ...`, `data: ...`, `ci: ...`,
  `fix(auth): ...`. A hook validates the format.
- Code comments and user-facing strings are in Chinese; keep that consistent when editing.
