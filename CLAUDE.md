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
full assay sheet CSR + S/A/V/G/Y/M, feeding both the CSR regression and the future G-correction fit).

**Data flow inside the core** (`blend_kit_rs/src/`):
`model.rs` (Coal/Spec/BlendRequest/BlendResult types, 8-indicator constant) →
`optimizer.rs` (builds the Clarabel LP: minimize Σ cif·xᵢ s.t. Σxᵢ=1 and per-spec weighted ≤/≥ bounds; also
computes slack + `binding` flags) →
result is post-processed into three business views: cost breakdown, physical orders, indicator check.
`seed.rs` loads the embedded master DB + status state machine; `predict.rs` is optional CSR regression.

**Frontend screens** (`doudou_blend/src/screens/`): 今日 `TodayScreen` (solve + cost/recipe/8-indicator view,
real purchase-qty input, export orders, save-to-history, input-summary panel), 煤池 `CoalPoolScreen`
(enable/hide + price + assay overrides), 合同 `ContractScreen` (quality specs), 历史 `HistoryScreen` (saved
blends + measured-CSR backfill), 我的 `MeScreen`. Tabs mount/unmount on switch (`App.tsx`
`{tab === ... && <Screen/>}`), so each screen reloads its data on entry — relevant for async backend reads.
The whole app sits behind a login gate: `App.tsx` renders `LoginScreen` until `tryLogin` (`auth.ts`) succeeds
— credentials are checked server-side and the session is an HttpOnly cookie, so no password ships in the
frontend bundle. Screens read master data through `master_loader.ts`'s `loadMaster()` module cache; call
`invalidateMaster()` after anything that changes master data.

**Master vs. user data split.** `coal_master.json` (embedded in `blend_kit_rs/data/`, served read-only via
`GET /api/master`) is the canonical coal set and is never mutated. User edits (enable/hide, price overrides,
assay overrides) persist separately: `localStorage` acts as a synchronous cache (`doudou_blend/src/storage.ts`)
and `cloudStorage.ts` mirrors it to PostgreSQL through `GET/PUT /api/storage` after login.

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
cargo clippy --release -- -D warnings
cargo fmt
```

**Web server** (`cd blend_kit_server`):
```bash
cargo test --locked
cargo clippy --release -- -D warnings
cargo run                                      # needs AUTH_* + DATABASE_URL env (see docs/railway-deployment.md)
```

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

## Conventions

- Commit messages: `<type>(<scope>): <desc>` in Chinese, e.g. `feat(coal-pool): ...`, `data: ...`, `ci: ...`,
  `fix(auth): ...`. A hook validates the format.
- Code comments and user-facing strings are in Chinese; keep that consistent when editing.
