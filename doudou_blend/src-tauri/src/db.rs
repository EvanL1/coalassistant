//! 数据库门面: 打开连接 / 初始化 schema / seed master.
//!
//! 线程模型: 移动端单实例, Tauri command 之间用 Mutex<Connection> 共享 (低频).
//! 不引入连接池避免依赖.
use crate::db_schema;
use crate::db_seed;
use rusqlite::Connection;
use std::path::Path;
use thiserror::Error;

// SeedReport 暂未暴露给前端, 加 #[allow] 保留供未来使用
#[allow(unused_imports)]
pub use db_seed::SeedReport;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("seed: {0}")]
    SeedFailed(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl serde::Serialize for DbError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// 打开 (或创建) 数据库文件, 初始化 schema, 跑 master seed.
/// 幂等: 重复调用安全, 不重复 seed (依赖 meta 表的 master_version 标记).
pub fn open_and_init(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(path)?;
    init_schema(&mut conn)?;
    db_seed::seed_master(&mut conn)?;
    Ok(conn)
}

fn init_schema(conn: &mut Connection) -> Result<(), DbError> {
    conn.execute_batch(db_schema::SCHEMA_V1)?;
    migrate_blend_history(conn)?;
    Ok(())
}

/// 幂等迁移: 给已有老库的 blend_history 补上后加的列.
/// 全新安装时 SCHEMA_V1 的 CREATE TABLE 已含这些列, 这里检测到存在即跳过.
/// 不引入迁移框架 (沿用 schema 的 IF NOT EXISTS 思路).
fn migrate_blend_history(conn: &Connection) -> Result<(), DbError> {
    add_column_if_missing(conn, "blend_history", "contract_name", "TEXT")?;
    add_column_if_missing(conn, "blend_history", "csr_measured", "REAL")?;
    Ok(())
}

/// 列不存在才 ALTER TABLE ADD COLUMN. table/column 是内部常量, 非用户输入.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    decl_type: &str,
) -> Result<(), DbError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    // PRAGMA table_info 第 1 列 (索引 1) 是列名
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<String>, _>>()?
        .iter()
        .any(|c| c == column);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl_type}"),
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    /// 全链路: open_and_init → schema → seed master → 表里能查到 4 主力煤.
    #[test]
    fn test_seed_writes_verified_coals() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let conn = open_and_init(&path).expect("初始化失败");

        // 验证 4 主力煤都在
        for name in &["临北", "古交浮精", "豹子沟", "大佛寺"] {
            let status: String = conn
                .query_row(
                    "SELECT status FROM mines WHERE name = ?1",
                    params![name],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| panic!("找不到 {}", name));
            assert_eq!(status, "verified", "{} 状态应为 verified", name);
        }

        // 验证临北的指标写进 mines 宽表 + 生成列 cif + region 拆成省/市
        let (s, fob, frt, cif, province, city): (
            Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<String>, Option<String>,
        ) = conn
            .query_row(
                "SELECT s, fob, frt, cif, province, city FROM mines WHERE name = ?1",
                params!["临北"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();
        assert_eq!(s, Some(2.0), "临北 S 应为 2.0");
        assert_eq!(cif, Some(fob.unwrap() + frt.unwrap()), "cif 生成列应等于 fob+frt");
        assert_eq!(province.as_deref(), Some("山西"), "临北 region 应拆出省=山西");
        assert_eq!(city.as_deref(), Some("吕梁"), "临北 region 应拆出市=吕梁");

        // 可信度写入 mine_field_confidence
        let conf_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mine_field_confidence
                 WHERE mine_id = (SELECT id FROM mines WHERE name = ?1)",
                params!["临北"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(conf_count >= 8, "临北可信度字段数 {} < 8", conf_count);

        // 验证默认合同已 seed
        let contract_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM contracts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(contract_count, 1);

        let spec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM contract_specs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(spec_count, 8, "默认合同应 8 条 spec");

        // master_version 写入
        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'master_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "2.2");
    }

    /// 幂等: 二次调用 open_and_init 不应重复插入合同, 不破坏数据.
    #[test]
    fn test_seed_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");

        // 第一次
        let _ = open_and_init(&path).unwrap();

        // 模拟用户修改: 给临北加一个 user_override
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute(
                "INSERT INTO user_overrides (coal_name, field, value, updated_at) VALUES ('临北', 'S', 1.95, '2026-05-15')",
                [],
            ).unwrap();
            conn.execute(
                "UPDATE contracts SET name = '用户自定义合同 A' WHERE is_default = 1",
                [],
            ).unwrap();
        }

        // 第二次 open_and_init
        let conn = open_and_init(&path).unwrap();

        // user_overrides 没被覆盖
        let override_val: f64 = conn
            .query_row(
                "SELECT value FROM user_overrides WHERE coal_name = '临北' AND field = 'S'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(override_val, 1.95, "用户 override 应被保留");

        // 合同改名也被保留 (因为 default_contract_seeded 标志阻止了重插)
        let contract_name: String = conn
            .query_row(
                "SELECT name FROM contracts WHERE is_default = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(contract_name, "用户自定义合同 A", "用户合同改名应保留");

        // 合同表仍只有 1 个
        let contract_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM contracts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(contract_count, 1, "幂等性: 不应重复插入合同");
    }

    /// 采集 + 回填往返: save → list → set_measured_csr → list.
    #[test]
    fn test_history_save_list_backfill_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let mut conn = open_and_init(&path).unwrap();

        let id = crate::db_queries::save_history(
            &mut conn,
            "2026-06-30T10:00:00.000Z",
            "默认合同",
            1234.5,
            Some(5000.0),
            r#"{"ok":true,"indicator_check":[]}"#,
        )
        .unwrap();
        assert!(id > 0, "save_history 应返回正的行 id");

        let list = crate::db_queries::list_history(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].contract_name, "默认合同");
        assert_eq!(list[0].cost_cif, 1234.5);
        assert_eq!(list[0].csr_measured, None, "未回填时 csr_measured 应为 None");

        crate::db_queries::set_measured_csr(&mut conn, id, 65.3).unwrap();
        let list = crate::db_queries::list_history(&conn).unwrap();
        assert_eq!(list[0].csr_measured, Some(65.3), "回填后应读到实测值");

        // 不存在的 id → NotFound
        assert!(
            crate::db_queries::set_measured_csr(&mut conn, 9999, 60.0).is_err(),
            "回填不存在的 id 应报错"
        );

        // 清空
        crate::db_queries::clear_history(&mut conn).unwrap();
        assert_eq!(crate::db_queries::list_history(&conn).unwrap().len(), 0, "清空后应为空");
    }

    /// 迁移幂等: 老库 (无 contract_name/csr_measured 列) 经 open_and_init 补列, 二次调用不报错.
    #[test]
    fn test_blend_history_migration_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("old.db");

        // 模拟老库: 只建旧版 blend_history (缺后加的两列)
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"CREATE TABLE blend_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    occurred_at TEXT NOT NULL,
                    contract_id INTEGER,
                    total_quantity REAL,
                    cost_cif REAL NOT NULL,
                    result_json TEXT NOT NULL,
                    note TEXT
                );"#,
            )
            .unwrap();
        }

        // 两次 open_and_init 都不报错 (迁移幂等)
        let _ = open_and_init(&path).expect("第一次迁移失败");
        let conn = open_and_init(&path).expect("第二次迁移失败");

        // 新列已补上
        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(blend_history)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert!(cols.contains(&"csr_measured".to_string()), "应补 csr_measured 列");
        assert!(cols.contains(&"contract_name".to_string()), "应补 contract_name 列");

        // 迁移后写入/读取正常
        let mut conn = conn;
        let id = crate::db_queries::save_history(
            &mut conn, "2026-06-30T11:00:00.000Z", "老库合同", 999.0, None, "{}",
        )
        .unwrap();
        crate::db_queries::set_measured_csr(&mut conn, id, 62.0).unwrap();
        let list = crate::db_queries::list_history(&conn).unwrap();
        assert_eq!(list[0].csr_measured, Some(62.0));
    }
}
