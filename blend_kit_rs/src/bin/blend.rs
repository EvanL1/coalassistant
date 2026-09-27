//! 豆哥配煤命令行: 让 agent / 脚本直接调用配煤核心, 与线上 `POST /api/solve` 同一套算法.
//!
//! ```text
//! blend solve  < request.json   求最低成本配方
//! blend eval   < request.json   验算: 请求须带 fixed_ratios (煤名 → 份数), 按给定配比出结果
//! blend master                  输出内置 master 数据 (与 GET /api/master 同源)
//! ```
//!
//! 标准输入读 BlendRequest JSON, 标准输出写 BlendResult JSON.
//! 退出码: 0 = 有结果 (ok=true); 1 = ok=false, 原因在 reason; 2 = 用法或 JSON 错误.

use std::io::Read;
use std::process::ExitCode;

const USAGE: &str = "用法: blend solve|eval < request.json  |  blend master";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("master") => {
            println!("{}", blend_kit::master_json());
            ExitCode::SUCCESS
        }
        Some(command @ ("solve" | "eval")) => run(command),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run(command: &str) -> ExitCode {
    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("读取标准输入失败: {error}");
        return ExitCode::from(2);
    }
    let request: serde_json::Value = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("请求不是合法 JSON: {error}");
            return ExitCode::from(2);
        }
    };
    // 两个子命令只差这一个字段; 分开写是为了让调用方的意图不会被字段有无悄悄改掉.
    let has_fixed = request
        .get("fixed_ratios")
        .is_some_and(|value| !value.is_null());
    if command == "eval" && !has_fixed {
        eprintln!("eval 需要请求里带 fixed_ratios (煤名 → 份数)");
        return ExitCode::from(2);
    }
    if command == "solve" && has_fixed {
        eprintln!("solve 求最优, 请求里不应带 fixed_ratios; 要验算请用 eval");
        return ExitCode::from(2);
    }

    let output = blend_kit::solve_json(&input);
    println!("{output}");
    let ok = serde_json::from_str::<serde_json::Value>(&output)
        .ok()
        .and_then(|result| result.get("ok")?.as_bool())
        .unwrap_or(false);
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
