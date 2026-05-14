//! 豆哥配煤 Tauri 后端入口.
//!
//! 模块拆分:
//!   db / db_schema / db_seed / db_queries  - SQLite 持久化
//!   commands                               - Tauri 暴露给前端的 API
mod db;
mod db_queries;
mod db_schema;
mod db_seed;

use std::sync::Mutex;

use db::DbError;
use db_queries::{CoalView, ContractView};
use rusqlite::Connection;
use serde::Serialize;
use tauri::Manager;

/// 全局应用状态: 共享数据库连接.
pub struct AppState {
    pub conn: Mutex<Connection>,
}

// ============================================================
// 通用 commands
// ============================================================

#[tauri::command]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
fn solve_blend(input_json: String) -> String {
    blend_kit::solve_json(&input_json)
}

// ============================================================
// 数据库相关 commands
// ============================================================

#[tauri::command]
fn db_status(state: tauri::State<AppState>) -> Result<DbStatus, DbError> {
    let conn = state.conn.lock().unwrap();
    let total_coals: i64 = conn.query_row("SELECT COUNT(*) FROM master_coals", [], |r| r.get(0))?;
    let enabled_coals: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM user_coal_prefs WHERE enabled = 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let contracts: i64 = conn.query_row("SELECT COUNT(*) FROM contracts", [], |r| r.get(0))?;
    let history: i64 = conn.query_row("SELECT COUNT(*) FROM blend_history", [], |r| r.get(0))?;
    let master_version: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'master_version'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(DbStatus {
        master_version,
        total_coals,
        enabled_coals,
        contracts,
        history,
    })
}

#[tauri::command]
fn list_coals(
    state: tauri::State<AppState>,
    status: Option<String>,
) -> Result<Vec<CoalView>, DbError> {
    let conn = state.conn.lock().unwrap();
    db_queries::list_coals(&conn, status.as_deref())
}

#[tauri::command]
fn get_coal(state: tauri::State<AppState>, name: String) -> Result<CoalView, DbError> {
    let conn = state.conn.lock().unwrap();
    db_queries::get_coal(&conn, &name)
}

#[tauri::command]
fn set_user_override(
    state: tauri::State<AppState>,
    coal_name: String,
    field: String,
    value: f64,
) -> Result<(), DbError> {
    let mut conn = state.conn.lock().unwrap();
    db_queries::set_user_override(&mut conn, &coal_name, &field, value)
}

#[tauri::command]
fn clear_user_override(
    state: tauri::State<AppState>,
    coal_name: String,
    field: String,
) -> Result<(), DbError> {
    let mut conn = state.conn.lock().unwrap();
    db_queries::clear_user_override(&mut conn, &coal_name, &field)
}

#[tauri::command]
fn set_today_price(
    state: tauri::State<AppState>,
    coal_name: String,
    fob: Option<f64>,
    frt: Option<f64>,
) -> Result<(), DbError> {
    let mut conn = state.conn.lock().unwrap();
    db_queries::set_today_price(&mut conn, &coal_name, fob, frt)
}

#[tauri::command]
fn set_enabled(
    state: tauri::State<AppState>,
    coal_name: String,
    enabled: bool,
) -> Result<(), DbError> {
    let mut conn = state.conn.lock().unwrap();
    db_queries::set_enabled(&mut conn, &coal_name, enabled)
}

#[tauri::command]
fn list_contracts(state: tauri::State<AppState>) -> Result<Vec<ContractView>, DbError> {
    let conn = state.conn.lock().unwrap();
    db_queries::list_contracts(&conn)
}

#[tauri::command]
fn get_active_contract(state: tauri::State<AppState>) -> Result<ContractView, DbError> {
    let conn = state.conn.lock().unwrap();
    db_queries::get_active_contract(&conn)
}

#[derive(Debug, Serialize)]
struct DbStatus {
    master_version: Option<String>,
    total_coals: i64,
    enabled_coals: i64,
    contracts: i64,
    history: i64,
}

// ============================================================
// 入口
// ============================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data = app
                .path()
                .app_data_dir()
                .expect("app_data_dir 不可用");
            let db_path = app_data.join("doudou_blend.db");
            let conn = db::open_and_init(&db_path).expect("数据库初始化失败");
            app.manage(AppState {
                conn: Mutex::new(conn),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            version,
            solve_blend,
            db_status,
            list_coals,
            get_coal,
            set_user_override,
            clear_user_override,
            set_today_price,
            set_enabled,
            list_contracts,
            get_active_contract,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
