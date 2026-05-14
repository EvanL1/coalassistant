//! 豆哥配煤 WASM 包装层.
//!
//! 暴露给浏览器的 JS API:
//!   - solveJson(input: string): string
//!   - getMasterJson(): string  (返回嵌入的 master 数据)
//!   - getVersion(): string
//!
//! 用 wasm-bindgen 生成 TypeScript 类型, 前端 import 即可.
use wasm_bindgen::prelude::*;

// 初始化 hook: 让 Rust panic 在浏览器 console 里可读
#[cfg(feature = "console_error_panic_hook")]
fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[cfg(not(feature = "console_error_panic_hook"))]
fn init_panic_hook() {}

/// 主求解入口. 输入 BlendRequest JSON, 输出 BlendResult JSON.
/// 完全等价于 Tauri 后端的 `solve_blend` command.
#[wasm_bindgen(js_name = solveJson)]
pub fn solve_json(input_json: &str) -> String {
    init_panic_hook();
    blend_kit::solve_json(input_json)
}

/// 返回嵌入的 master 数据库 JSON.
/// 前端首次启动时调用, 显示 73+ 煤种 + 默认合同.
#[wasm_bindgen(js_name = getMasterJson)]
pub fn get_master_json() -> String {
    init_panic_hook();
    // 直接读嵌入的 master JSON 原文 (避免反序列化再序列化的开销)
    include_str!("../../blend_kit_rs/data/coal_master.json").to_string()
}

/// 返回 crate 版本, 给前端做版本检查用.
#[wasm_bindgen(js_name = getVersion)]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
