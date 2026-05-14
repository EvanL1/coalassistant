//! SQLite schema 定义.
//!
//! 设计原则:
//!   - master_* 表是 blend_kit master 数据的镜像, 由 seed 写入, 应用层不修改
//!   - user_* 表是用户私有数据, 通过 source_id 引用 master
//!   - 读 coal 时 LEFT JOIN: user 层优先, master 层 fallback
//!     这样 master 升级 (新版本) 不会覆盖用户的修改
//!
//! 不使用迁移框架. 当前 schema v1 用 INSERT OR IGNORE 兼容多次启动.
pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- ============================================================
-- Master 层: 煤种字典 (只读, 由 blend_kit master JSON 同步)
-- ============================================================
CREATE TABLE IF NOT EXISTS master_coals (
    name           TEXT PRIMARY KEY,         -- 煤名 (作为天然主键)
    region         TEXT,
    coal_type      TEXT,
    status         TEXT NOT NULL,            -- verified/active/draft/incomplete/archived
    master_version TEXT NOT NULL,            -- 写入时的 master JSON 版本
    note           TEXT
);

CREATE TABLE IF NOT EXISTS master_indicators (
    coal_name    TEXT NOT NULL REFERENCES master_coals(name) ON DELETE CASCADE,
    field        TEXT NOT NULL,              -- S/A/V/G/Y/petro/CSR/M/fob/frt
    value        REAL NOT NULL,
    confidence   TEXT,                       -- high/medium/low (可选)
    PRIMARY KEY (coal_name, field)
);

-- ============================================================
-- User 层: 用户私有数据 (override + 启用状态)
-- ============================================================
-- 用户对单个字段的 override.
-- master 升级时不会动这张表, 用户的改动得以保留.
CREATE TABLE IF NOT EXISTS user_overrides (
    coal_name    TEXT NOT NULL REFERENCES master_coals(name) ON DELETE CASCADE,
    field        TEXT NOT NULL,
    value        REAL NOT NULL,
    updated_at   TEXT NOT NULL,              -- ISO8601
    PRIMARY KEY (coal_name, field)
);

-- 用户对煤的偏好: 是否启用 / 今日价格 / 备注
CREATE TABLE IF NOT EXISTS user_coal_prefs (
    coal_name        TEXT PRIMARY KEY REFERENCES master_coals(name) ON DELETE CASCADE,
    enabled          INTEGER NOT NULL DEFAULT 0,      -- 0/1
    today_fob        REAL,                            -- 今日出厂价 (null = 用 master 默认)
    today_frt        REAL,                            -- 今日运费
    price_updated_at TEXT,                            -- ISO8601, 最近一次价格更新
    note             TEXT                             -- 用户备注 (供应商联系人等)
);

-- ============================================================
-- 合同模板
-- ============================================================
CREATE TABLE IF NOT EXISTS contracts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT UNIQUE NOT NULL,
    is_default  INTEGER NOT NULL DEFAULT 0,           -- 1 = master 自带默认
    is_active   INTEGER NOT NULL DEFAULT 0,           -- 1 = 当前激活 (只能一个)
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS contract_specs (
    contract_id INTEGER NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    indicator   TEXT NOT NULL,
    direction   TEXT NOT NULL,                        -- Upper/Lower/Range
    min_val     REAL,
    max_val     REAL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (contract_id, indicator)
);

-- ============================================================
-- 历史方案
-- ============================================================
CREATE TABLE IF NOT EXISTS blend_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at     TEXT NOT NULL,                    -- ISO8601 时间戳
    contract_id     INTEGER REFERENCES contracts(id),
    total_quantity  REAL,
    cost_cif        REAL NOT NULL,
    result_json     TEXT NOT NULL,                    -- 完整 BlendResult JSON, 便于追溯
    note            TEXT
);

CREATE INDEX IF NOT EXISTS idx_history_occurred_at ON blend_history(occurred_at DESC);
"#;
