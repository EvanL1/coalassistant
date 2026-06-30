# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

豆哥配煤 (Doudou Blend) — a coking-coal blend optimizer. It takes a pool of coals (each with 8 quality
indicators + an FOB/FRT price split) plus quality specs, and solves a linear program for the cheapest blend
ratio that satisfies every constraint. Ships as a mobile/desktop Tauri app **and** a browser web app from one
React codebase.

## Repository layout (three independent Rust crates — NOT a cargo workspace)

| Path | Crate / package | Role |
|------|-----------------|------|
| `blend_kit_rs/` | `blend_kit` | Pure LP core. Clarabel solver, embedded master data. Zero Tauri/WASM/web deps. |
| `blend_kit_wasm/` | `blend_kit_wasm` | Thin `wasm-bindgen` wrapper. Re-exposes the core to JS. Depends on `blend_kit` by path. |
| `doudou_blend/src-tauri/` | `doudou_blend` | Tauri 2.0 native backend. SQLite persistence + Tauri commands. Depends on `blend_kit` by path. |
| `doudou_blend/src/` | (npm) `doudou_blend` | React 19 + TypeScript + Vite frontend. |

Each crate has its own `Cargo.toml` and `Cargo.lock`; build them from inside their own directory.

## Architecture — the parts that span files

**One algorithm, three callers.** The entire core is reached through a single string-in/string-out function,
`blend_kit::solve_json(&str) -> String` (`blend_kit_rs/src/lib.rs`). It is invoked three ways:
- directly in tests/examples,
- via `wasm-bindgen` as `solveJson` (`blend_kit_wasm/src/lib.rs`),
- via a `#[tauri::command] solve_blend` (`doudou_blend/src-tauri/src/lib.rs`).

Keeping every boundary at "JSON string in, JSON string out" is what lets the same React app run on two
runtimes. Don't add typed cross-boundary APIs; extend the JSON request/result shapes instead.

**Dual backend, picked at runtime.** `doudou_blend/src/backend.ts` detects the `__TAURI_INTERNALS__` global:
present → route to Tauri IPC; absent → load the WASM module directly. Both expose the *same async* interface
(`solveJson` / `getMasterJson` / `getVersion`), so screen code never branches on runtime. When adding a backend
capability, add it to **both** `makeTauriBackend` and `makeWasmBackend`.

**Data flow inside the core** (`blend_kit_rs/src/`):
`model.rs` (Coal/Spec/BlendRequest/BlendResult types, 8-indicator constant) →
`optimizer.rs` (builds the Clarabel LP: minimize Σ cif·xᵢ s.t. Σxᵢ=1 and per-spec weighted ≤/≥ bounds; also
computes slack + `binding` flags) →
result is post-processed into three business views: cost breakdown, physical orders, indicator check.
`seed.rs` loads the embedded master DB + status state machine; `predict.rs` is optional CSR regression.

**Master vs. user data split.** `coal_master.json` (embedded in `blend_kit_rs/data/`, served read-only) is the
canonical coal set and is never mutated. User edits (enable/hide, price overrides, assay overrides) persist
separately: `localStorage` on web (`doudou_blend/src/storage.ts`) and SQLite tables `user_coal_prefs` /
`user_overrides` on native (`doudou_blend/src-tauri/src/db_*.rs`). The SQLite DB self-seeds idempotently on
first open, keyed by `meta.master_version`.

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

**WASM package** (required before any frontend build — see gotcha below):
```bash
cd blend_kit_wasm
wasm-pack build --target web --out-dir pkg --release
node test_wasm_solve.mjs                       # smoke-test the built .wasm
```

**Frontend / Tauri** (`cd doudou_blend`, needs Node 22+):
```bash
npm install
npm run dev          # web dev server (Vite, :1420) — WASM pkg must already be built
npm run build        # tsc && vite build → dist/
npm run tauri dev    # native dev
npm run tauri build  # native bundle
npx @tauri-apps/cli android init && npx @tauri-apps/cli android build --apk --target aarch64
```

## Critical gotchas

- **Always run `wasm-pack build` before any `npm install`/`build`/`tauri build`.** `package.json` declares
  `"blend-kit-wasm": "file:../blend_kit_wasm/pkg"`, and that `pkg/` directory is gitignored. Even Tauri builds
  (which use IPC, not WASM, at runtime) fail to resolve the TypeScript import until `pkg/` exists. This is the
  #1 source of "fresh checkout won't build". Every CI job builds WASM first for exactly this reason.
- **`doudou_blend/src/types.ts` is a hand-maintained mirror of the Rust schema.** Change a struct in
  `blend_kit_rs/src/model.rs` (or its serde shape) and you must update `types.ts` to match — there is no
  codegen for the app data types. The 8-indicator list `["S","A","V","G","Y","petro","CSR","M"]` and its
  ordering are duplicated in both `model.rs` (`INDICATORS`) and `types.ts` (`INDICATOR_ORDER`).
- **`blend_kit_wasm` disables `wasm-opt`** (bundled version is too old for Rust 1.82+ bulk-memory). Don't
  re-enable it. Size is already controlled via `opt-level="z"` + LTO.
- **`Direction::Upper` ignores `spec.min`; `Direction::Lower` ignores `spec.max`** by design — only `Range`
  uses both bounds. There are regression tests pinning this (`test_direction_upper_ignores_min`).
- **`npm run dev` needs `server.fs.allow` to include the repo root** (`vite.config.ts` sets `[".", ".."]`).
  `blend-kit-wasm` is a `file:../blend_kit_wasm/pkg` dep, so its `.wasm` lives outside `doudou_blend/` and Vite's
  dev file-serving allowlist rejects it by default → `WebAssembly.compile` fails ("HTTP status code is not ok")
  and the app white-screens. `vite build` bundles the wasm so production/CI never catch it — only the dev server does.

## CI / release

- `ci.yml` (push/PR): tests + builds the `blend_kit_rs` examples + clippy/fmt (warn-only). Tauri is **not**
  tested in CI (system deps); leave end-to-end builds to release.
- `release.yml` (push a `v*.*.*` tag): builds Android APK (arm64 + armv7) and macOS universal DMG, attaches to
  a published GitHub Release. To cut a release: `git tag v0.1.0 && git push origin v0.1.0`.
- `deploy-web.yml` (push to `main`): builds WASM → Vite bundle → GitHub Pages at `/coalassistant/` base path.
  `wrangler.jsonc` also exists for an alternate Cloudflare Workers static deploy.

## Conventions

- Commit messages: `<type>(<scope>): <desc>` in Chinese, e.g. `feat(coal-pool): ...`, `data: ...`, `ci: ...`,
  `fix(auth): ...`. A hook validates the format.
- Code comments and user-facing strings are in Chinese; keep that consistent when editing.
