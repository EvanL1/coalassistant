# Repository Guidelines

## Project Structure & Module Organization

This repository contains three independent Rust crates, not a Cargo workspace. `blend_kit_rs/` is the business core: models, optimizer, prediction, embedded data, tests, and examples. `blend_kit_wasm/` wraps that core for browsers through `wasm-bindgen`. `doudou_blend/` contains the React 19/TypeScript UI in `src/` and the Tauri 2/SQLite backend in `src-tauri/`. Design references are in `docs/` and `mockup/`; the live native schema is `doudou_blend/src-tauri/src/db_schema.rs`.

## Build, Test, and Development Commands

Run commands from the relevant package directory:

```bash
cd blend_kit_rs && cargo test --release
cargo run --release --example master_demo
cargo fmt -- --check && cargo clippy --release -- -D warnings

cd ../blend_kit_wasm
wasm-pack build --target web --out-dir pkg --release
node test_wasm_solve.mjs

cd ../doudou_blend
npm install
npm run dev             # Vite web app on port 1420
npm run build           # Type-check and create dist/
npm run tauri dev       # Native desktop development
```

Build the WASM package before installing or building the frontend because `package.json` references the generated `blend_kit_wasm/pkg` directory.

## Coding Style & Naming Conventions

Use `rustfmt` defaults and idiomatic Rust `snake_case`; keep tests near their modules under `#[cfg(test)]`. TypeScript uses strict compiler checks, two-space indentation, `PascalCase` for React components, and `camelCase` for variables and functions. Keep comments and user-facing text in Chinese. Preserve the JSON-string boundary exposed by `blend_kit::solve_json`; update both Tauri and WASM adapters when adding backend capabilities.

## Testing Guidelines

Name Rust tests descriptively with a `test_` prefix, for example `test_basic_solve`. Run core tests for every algorithm or schema-shape change and add regression coverage for constraints and edge cases. Run the WASM smoke test after rebuilding `pkg/`. There is no frontend unit-test framework currently; use `npm run build` as the required type/build check and manually exercise affected screens.

## Commit & Pull Request Guidelines

Follow the existing Conventional Commit style, usually with Chinese descriptions: `feat(today): 添加价格走势`, `fix(auth): 修复登录状态`, or `docs: 更新说明`. Keep commits scoped and avoid mixing generated artifacts with unrelated changes. Pull requests should explain user impact, list verification commands, link relevant issues or design documents, and include screenshots for UI changes. Ensure CI-facing Rust tests, formatting, Clippy, and the frontend build pass before requesting review.

## Data Synchronization

`blend_kit_rs/data/coal_master.json` is the sole Master source for WASM, Tauri, and SQLite seed. After changing shared Rust models or versions, update TypeScript as needed and run `npm run check:consistency` from `doudou_blend/`.
