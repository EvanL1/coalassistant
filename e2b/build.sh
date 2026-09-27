#!/usr/bin/env bash
# 构建 doudou-blend E2B 模板: 沙箱里预装 blend 命令行.
#
# 用法: e2b/build.sh          (需要 E2B_API_KEY)
#
# 构建上下文在临时目录里组装, 只放 blend_kit_rs 的源码与数据:
# 直接用仓库根目录会把 1GB+ 的 blend_kit_rs/target 整个上传.
# 构建完跑 e2b/smoke.py, 在真沙箱里验证 blend 能用、结果对.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPLATE="doudou-blend"

if [ -z "${E2B_API_KEY:-}" ]; then
    echo "ERROR: E2B_API_KEY 未设置" >&2
    exit 1
fi

CTX="$(mktemp -d)"
trap 'rm -rf "$CTX"' EXIT

mkdir -p "$CTX/blend_kit_rs"
cp -R "$ROOT/blend_kit_rs/Cargo.toml" "$ROOT/blend_kit_rs/Cargo.lock" \
      "$ROOT/blend_kit_rs/src" "$ROOT/blend_kit_rs/data" "$ROOT/blend_kit_rs/examples" \
      "$CTX/blend_kit_rs/"
cp "$ROOT/e2b/e2b.Dockerfile" "$ROOT/e2b/README.sandbox.md" "$CTX/"

echo "==> 构建上下文: $(du -sh "$CTX" | cut -f1)"
e2b template create "$TEMPLATE" \
    --path "$CTX" \
    --dockerfile e2b.Dockerfile \
    --cpu-count 2 \
    --memory-mb 2048

echo "==> 冒烟测试"
uv run --with e2b python "$ROOT/e2b/smoke.py" "$TEMPLATE"
