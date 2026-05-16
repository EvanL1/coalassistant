#!/usr/bin/env bash
# 编 blend_kit_rs 到 cdylib, 给 Flutter Dart FFI 用.
#
# 用法:
#   bash build_rust.sh host                # 本机 (Mac/Linux/Windows)
#   bash build_rust.sh android arm64
#   bash build_rust.sh android armv7
#   bash build_rust.sh ios arm64
#   bash build_rust.sh ohos arm64           # 鸿蒙 NEXT
#
# 产物落到: doudou_blend_flutter/build/<platform-arch>/lib*.{so,dylib,dll}
#
# 前置: blend_kit_rs 的 src/lib.rs 必须暴露 extern "C" fn blend_solve.
# 见 native/README.md.

set -euo pipefail

cd "$(dirname "$0")"
FLUTTER_DIR="$PWD"
RUST_DIR="$PWD/../blend_kit_rs"

if [[ ! -f "$RUST_DIR/Cargo.toml" ]]; then
  echo "找不到 blend_kit_rs/Cargo.toml: $RUST_DIR" >&2
  exit 1
fi

PLATFORM="${1:-host}"
ARCH="${2:-arm64}"

case "$PLATFORM" in
  host)
    cd "$RUST_DIR"
    cargo build --release
    OUT="$FLUTTER_DIR/build/host"
    mkdir -p "$OUT"
    case "$(uname -s)" in
      Darwin) cp target/release/libblend_kit_rs.dylib "$OUT/" ;;
      Linux)  cp target/release/libblend_kit_rs.so "$OUT/" ;;
      MINGW*|MSYS*|CYGWIN*) cp target/release/blend_kit_rs.dll "$OUT/" ;;
      *) echo "未知 host OS: $(uname -s)" >&2; exit 1 ;;
    esac
    ;;
  android)
    case "$ARCH" in
      arm64) TARGET="aarch64-linux-android" ;;
      armv7) TARGET="armv7-linux-androideabi" ;;
      *) echo "未支持 arch: $ARCH" >&2; exit 1 ;;
    esac
    cd "$RUST_DIR"
    cargo build --release --target "$TARGET"
    OUT="$FLUTTER_DIR/build/android-$ARCH"
    mkdir -p "$OUT"
    cp "target/$TARGET/release/libblend_kit_rs.so" "$OUT/"
    ;;
  ios)
    case "$ARCH" in
      arm64) TARGET="aarch64-apple-ios" ;;
      *) echo "iOS 只支持 arm64" >&2; exit 1 ;;
    esac
    cd "$RUST_DIR"
    cargo build --release --target "$TARGET"
    OUT="$FLUTTER_DIR/build/ios-$ARCH"
    mkdir -p "$OUT"
    cp "target/$TARGET/release/libblend_kit_rs.a" "$OUT/"
    ;;
  ohos)
    # 鸿蒙 NEXT 关键平台. 需要装好鸿蒙 NDK + 配 ~/.cargo/config.toml linker.
    # 详见 native/README.md.
    case "$ARCH" in
      arm64) TARGET="aarch64-unknown-linux-ohos" ;;
      armv7) TARGET="armv7-unknown-linux-ohoseabi" ;;
      *) echo "未支持 arch: $ARCH" >&2; exit 1 ;;
    esac
    cd "$RUST_DIR"
    rustup target add "$TARGET" 2>/dev/null || true
    cargo build --release --target "$TARGET"
    OUT="$FLUTTER_DIR/build/ohos-$ARCH"
    mkdir -p "$OUT"
    cp "target/$TARGET/release/libblend_kit_rs.so" "$OUT/"
    echo
    echo "下一步: 把产物拷进鸿蒙工程:"
    echo "  mkdir -p $FLUTTER_DIR/ohos/entry/libs/arm64-v8a"
    echo "  cp $OUT/libblend_kit_rs.so $FLUTTER_DIR/ohos/entry/libs/arm64-v8a/"
    ;;
  *)
    echo "未知 platform: $PLATFORM" >&2
    echo "用法: $0 [host|android|ios|ohos] [arm64|armv7]" >&2
    exit 1
    ;;
esac

echo "构建完成: $OUT"
ls -lh "$OUT"
