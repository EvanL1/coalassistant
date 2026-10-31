# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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

Repo-root loose files: `schema.sql` / `mines.db` / `migrate_json_to_sqlite.mjs` were the design artifacts for
the mines wide-table migration in the (now removed) Tauri/SQLite backend. They are **orphaned** — nothing
builds or reads them. `mockup/` is the original HTML visual mockup of the six screens.

## Architecture — the parts that span files

**One algorithm, two callers.** The entire core is reached through a single string-in/string-out function,
`blend_kit::solve_json(&str) -> String` (`blend_kit_rs/src/lib.rs`). It is invoked:
- directly in tests/examples and by the server (`POST /api/solve`),
- via `wasm-bindgen` as `solveJson` (`blend_kit_wasm/src/lib.rs`) — built, but not wired into the app.

Keeping every boundary at "JSON string in, JSON string out" is what keeps the core independent of transport.
Don't add typed cross-boundary APIs; extend the JSON request/result shapes instead.

**Single backend seam.** `doudou_blend/src/backend.ts` exposes one async interface (`solveJson` /
`getMasterJson` / `getVersion`, plus blend history: `saveHistory` / `countHistory` / `listHistory` /
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
each coal is priced from the benchmark of the same coal type → same form (`Coal.form`: 精煤/筛选精煤/浮选精煤…; hard layers, each applied only if such a benchmark exists) whose premiums differ least — region is only a soft preference (`price_factor.region_penalty` points added to other-region benchmarks), because forcing a sparse local benchmark mis-prices by hundreds — `price = benchmark × Q(coal)/Q(benchmark)`,
falling back to `market_price`. Own JSON entry `estimate_json` (server `POST /api/price`, CLI `blend price`);
`price_factor.rs` is only the solver glue (unpriced coals enter the LP at the estimate; `coal_values` attached after).
Benchmarks live in three layers merged by the server (`blend_kit_server/src/benchmarks.rs`): master defaults + the
`price_benchmarks` table (admin-entered, latest date per name wins) + the **Mysteel coking-coal index** (MCCI,
`blend_kit_server/src/mcci.rs`): the server pulls 7 fixed-spec series daily after 17:30 Beijing into `mcci_daily`
(signed request copied from the public index page; `MCCI_FETCH=off` disables), and each series' latest value becomes a
benchmark whose props are the standard product in master `price_factor.index_series`. Older quotes are drifted into
`current_price` by the same-coal-type MCCI series (nearest S grade) when it can, else by CCP. CCP is index points
(2017-01 = 1000) — never use it as a price. Master-only `price_factor` keys (`*_note`, `index_series`) are stripped before
reaching the core (`MASTER_ONLY_KEYS`, mirrored in `doudou_blend/src/benchmarks.ts`).
`blend_kit_server/src/coal_prices.rs` 在服务启动后每小时重算当天 (北京时间) 的共享煤库估价快照, 并在启动时与每过零点
按"当天能拿到的指数与报价" (`benchmarks::as_of`) 补算今天以前缺的 (日期, 煤) —— 存下就不改, 只补空 (`source` 区分
live / backfill); `GET /api/coal-prices` 返回最新日与前一日价格, `GET /api/coal-prices/history?name=` 返回单煤全部历史;
煤池卡片据此显示估价与涨跌, 点估价行看走势; 用户修改或新增的煤按当前值现算.
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
rounding breaks a spec. Blend values come from the root LP; price-robustness re-solves also go through the shovel
wrapper. The shovel size is a per-user setting (我的 page, `web_user_state.shovel_tons`, NULL = default 3 t; saved with
COALESCE so pre-deploy clients can't reset it). Absent = byte-identical.

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

**估价与询价精算.** 求解默认全按估价: `domain/resolvedCoal.ts` 的 `toEstimateCoal` 与 `today.rs` 都不读
master 或偏好里的价格键, 启用煤放进 `price_factor.extra_coals`, `coals: []`, 打开 `estimate_unpriced`.
唯一例外是今日页「询价精算」: 用户当场填几种煤的出厂价与运费, 只用填了出厂价的煤求解 (至少两种),
按到厂价 = 出厂价 + 运费计算. 当场改的指标只进本次请求与存档, 询价不进煤池、不写本地或云端偏好、不跨次沿用;
下次进入时输入为空, 最近询价只作 placeholder. 每次精算按批存档到 `coal_quotes`, 同一会话后一次覆盖前一次.
存档估价由 `blend_kit_server/src/quotes.rs` 按这批指标与当天基准重算, 运费不进估价; 没匹配基准记 NULL,
不拿本批报价反推的价格做对比. 存档失败不影响精算结果, 精算不可行仍存档. 后台「询价对比」按煤种汇总
报价/估价中位数与差额中位数, 可删除填错的记录, 供以后校准估价系数. 历史以结果中的正数 fob 派生「询价」标记.
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
cargo test --release                          # all tests (live in lib.rs + per-module #[cfg(test)])
cargo test --release test_basic_solve         # a single test by name
cargo run --release --example master_demo     # demo against the full master DB
cargo run --release --example demo
cargo run --release --bin blend -- eval < request.json   # 命令行 (给 agent/脚本): solve 求最优 / eval 按 fixed_ratios 验算 / master 输出内置数据
cargo clippy --release -- -D warnings
cargo fmt
```

**Web server** (`cd blend_kit_server`):
```bash
cargo test --locked
cargo clippy --release -- -D warnings
cargo run                                      # needs AUTH_* + DATABASE_URL env (see docs/railway-deployment.md)
cargo run --release --bin doudou -- --profile admin login <账号>   # 线上命令行, 见下
```

**线上命令行 `doudou`** (`blend_kit_server/src/bin/doudou.rs`): 登录线上服务, 走 `/api/*`, 与网页同一套数据
(煤库覆盖值、后台设的 k、历史)。`--profile` 切换账号 (普通用户 / 管理员), 档案存 `~/.config/doudou/profiles.json`
(600, 只存会话 cookie)。密码读 `DOUDOU_PASSWORD` 或终端输入。命令: `login/logout/whoami/profiles`、`solve/eval`、
`master`、`history [measured <id>]`、`admin settings [--k|--off]|calibration|csr-model|keys|overrides`、`api <METHOD> <路径>`。
`blend` 只在本机用内置 master 离线算; 要和用户看到的一致, 用 `doudou`。

**MCP 入口 `/mcp`** (手机 Agent, 如 Kimi): 无状态 JSON-RPC, 工具 `today_blend` / `check_ratios` / `find_coal` /
`get_contract`。鉴权用后台生成的 **mcp 用途**密钥 (`data_api_keys.scope`, 只读+求解, 写不了煤库; data 用途的写煤库密钥
进不来)。求解请求由 `src/today.rs` 按线上保存的合同与煤池组装 —— 规则对照前端 `domain/resolvedCoal.ts`,
改了前端的煤池规则要同步改这里。接入说明见 `docs/mcp-agent.md`。

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
```

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

- `ci.yml` (push/PR) is the only workflow: consistency + master-data checks, frontend test/build,
  `blend_kit` tests + examples, `blend_kit_server` tests, fmt and clippy.
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
