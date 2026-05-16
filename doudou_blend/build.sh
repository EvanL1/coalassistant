#!/usr/bin/env bash
#
# Cloudflare Workers Builds 用的 build pipeline.
#
# CF Builds 环境只预装 Node, 没有 Rust/wasm-pack, 所以这里全自己装.
# 假设 cwd = doudou_blend/ (Root directory 配置为 doudou_blend).
#
# 跟 .github/workflows/deploy-web.yml (GH Pages 部署) 对应, 但 GH Actions
# Ubuntu runner 已预装 Rust 所以只需装 wasm-pack, CF Builds 啥都没有.
#
# 本地也能跑: cd doudou_blend && bash build.sh

set -euo pipefail

echo "▶ Step 1/4: Ensure Rust toolchain"
if ! command -v cargo > /dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown

echo "▶ Step 2/4: Ensure wasm-pack"
if ! command -v wasm-pack > /dev/null 2>&1; then
  curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
fi
export PATH="$HOME/.cargo/bin:$PATH"

echo "▶ Step 3/4: Build WASM package (blend_kit_wasm → pkg/)"
( cd ../blend_kit_wasm && wasm-pack build --target web --out-dir pkg --release )

echo "▶ Step 4/4: Install deps + Vite build"
npm install
npm run build

echo "✓ Build done. dist/ ready for wrangler deploy."
