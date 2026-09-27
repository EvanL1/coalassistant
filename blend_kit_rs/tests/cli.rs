//! `blend` 命令行: agent 在沙箱里直接调用配煤核心的入口.
//! 契约: 标准输入读 BlendRequest JSON, 标准输出写 BlendResult JSON;
//! 退出码 0 = 有结果, 1 = ok=false (原因在 reason), 2 = 用法或 JSON 错误.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_blend"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("启动 blend");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn coal(name: &str, s: f64, g: f64, fob: f64) -> serde_json::Value {
    serde_json::json!({
        "name": name, "fob": fob, "frt": 0.0,
        "props": {"S": s, "A": 10.0, "V": 24.0, "G": g, "Y": 18.0, "petro": 0.1, "CSR": 60.0, "M": 10.0}
    })
}

fn body(fixed: Option<serde_json::Value>, s_max: f64) -> String {
    let mut request = serde_json::json!({
        "coals": [coal("甲", 0.5, 90.0, 2000.0), coal("乙", 2.0, 70.0, 1500.0)],
        "specs": [{"indicator": "S", "direction": "Upper", "max": s_max, "enabled": true}],
        "total_quantity": null,
        "truncate_decimal": false
    });
    if let Some(fixed) = fixed {
        request["fixed_ratios"] = fixed;
    }
    request.to_string()
}

#[test]
fn test_eval_returns_result_for_given_shares() {
    let (code, stdout, stderr) = run(
        &["eval"],
        &body(Some(serde_json::json!({"甲": 1, "乙": 3})), 2.5),
    );
    assert_eq!(code, 0, "{stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["ok"], true);
    let share = result["recipe"]["乙"].as_f64().unwrap();
    assert!((share - 0.75).abs() < 1e-12);
}

#[test]
fn test_solve_returns_optimum() {
    let (code, stdout, stderr) = run(&["solve"], &body(None, 1.5));
    assert_eq!(code, 0, "{stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["ok"], true);
}

#[test]
fn test_infeasible_exits_1_with_json_reason() {
    let (code, stdout, _) = run(&["solve"], &body(None, 0.1));
    assert_eq!(code, 1);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert!(result["reason"].is_string());
}

#[test]
fn test_eval_without_fixed_ratios_is_a_usage_error() {
    let (code, _, stderr) = run(&["eval"], &body(None, 2.5));
    assert_eq!(code, 2);
    assert!(stderr.contains("fixed_ratios"), "{stderr}");
}

#[test]
fn test_bad_json_and_unknown_command_are_usage_errors() {
    assert_eq!(run(&["solve"], "{不是 JSON").0, 2);
    assert_eq!(run(&["frobnicate"], "").0, 2);
}

/// 只验结构: master 内容归数据自检管, 这里不断言任何煤.
#[test]
fn test_master_prints_the_embedded_master_json() {
    let (code, stdout, _) = run(&["master"], "");
    assert_eq!(code, 0);
    let master: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(master["coals"].is_array());
}
