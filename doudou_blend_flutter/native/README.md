# blend_kit_rs cdylib 构建

把 `/home/user/coalassistant/blend_kit_rs` 编成各平台的 cdylib, 给 Flutter
通过 Dart FFI 调用. 替代纯 Dart fallback solver, 跑真正的 LP 配煤.

## 前置: blend_kit_rs 暴露 C ABI

当前 `blend_kit_rs` 只暴露了 Rust API + WASM bindings, 还没 C ABI. 第一次
构建前需要在 `blend_kit_rs/src/lib.rs` 加 extern "C" 包装:

```rust
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn blend_solve(input_json: *const c_char) -> *mut c_char {
    let input = unsafe { CStr::from_ptr(input_json).to_string_lossy().into_owned() };
    let result = match serde_json::from_str::<crate::BlendRequest>(&input) {
        Ok(req) => crate::solve(&req),
        Err(e) => crate::BlendResult::error(&format!("invalid input: {}", e)),
    };
    let out = serde_json::to_string(&result).unwrap_or_else(|e| {
        format!(r#"{{"ok":false,"reason":"serialize failed: {}"}}"#, e)
    });
    CString::new(out).unwrap().into_raw()
}

#[no_mangle]
pub extern "C" fn blend_free_string(s: *mut c_char) {
    if s.is_null() { return; }
    unsafe { let _ = CString::from_raw(s); }
}
```

并在 `blend_kit_rs/Cargo.toml` 加:

```toml
[lib]
crate-type = ["cdylib", "rlib"]
```

## 构建各平台

用 `doudou_blend_flutter/build_rust.sh`:

```bash
# 桌面 (host 平台, 给 flutter desktop / 跑测试用)
bash build_rust.sh host

# Android (给标准 Flutter Android 用, 鸿蒙不走这里)
bash build_rust.sh android arm64
bash build_rust.sh android armv7

# iOS
bash build_rust.sh ios arm64

# 鸿蒙 NEXT — 关键平台
bash build_rust.sh ohos arm64
```

产物在 `doudou_blend_flutter/build/<platform-arch>/libblend_kit_rs.{so,dylib,dll}`.

## 鸿蒙 target 工具链

鸿蒙 Rust target 是 `aarch64-unknown-linux-ohos`, 需要装鸿蒙 NDK + 配
`~/.cargo/config.toml`:

```toml
[target.aarch64-unknown-linux-ohos]
linker = "/path/to/ohos-ndk/native/llvm/bin/aarch64-unknown-linux-ohos-clang"
ar = "/path/to/ohos-ndk/native/llvm/bin/llvm-ar"
```

NDK 在 DevEco Studio 装鸿蒙 SDK 时一起装的 (`~/Library/Huawei/Sdk/HarmonyOS-NEXT-DB1/openharmony/native` 之类).

更细的鸿蒙 Rust 工具链配法见
<https://gitee.com/openharmony/docs/blob/master/zh-cn/application-dev/quick-start/start-with-rust.md>.

## 把 .so 塞进鸿蒙 .hap

```bash
mkdir -p doudou_blend_flutter/ohos/entry/libs/arm64-v8a
cp doudou_blend_flutter/build/ohos-arm64/libblend_kit_rs.so \
   doudou_blend_flutter/ohos/entry/libs/arm64-v8a/
```

DevEco 打包 `.hap` 时自动收 `libs/arm64-v8a/` 里所有 .so. Dart 端通过
`DynamicLibrary.open('libblend_kit_rs.so')` 就能加载.
