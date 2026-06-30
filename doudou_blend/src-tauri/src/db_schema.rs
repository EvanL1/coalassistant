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
-- Master 层: 煤源字典 (只读, 由 blend_kit master JSON 同步)
-- 宽表: 一行一个煤源, 8 项指标各占一列, 严格 CHECK 约束.
-- ============================================================
CREATE TABLE IF NOT EXISTS mines (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,                  -- 煤名 (唯一键)
    coal_type   TEXT,                                     -- 煤种
    status      TEXT    NOT NULL DEFAULT 'incomplete'
                CHECK (status IN ('verified','active','draft','incomplete','archived')),

    -- 位置 (province/city 由 master region 拆分; county/mine_name/经纬度 后补, seed 不覆盖)
    province    TEXT,
    city        TEXT,
    county      TEXT,
    mine_name   TEXT,
    lat         REAL    CHECK (lat BETWEEN -90  AND 90),
    lng         REAL    CHECK (lng BETWEEN -180 AND 180),

    -- 煤质指标 (8 项, 缺测为 NULL)
    s           REAL    CHECK (s     BETWEEN 0 AND 100),  -- 硫 %
    a           REAL    CHECK (a     BETWEEN 0 AND 100),  -- 灰 %
    v           REAL    CHECK (v     BETWEEN 0 AND 100),  -- 挥发 %
    g           REAL    CHECK (g     BETWEEN 0 AND 100),  -- 粘结指数 G
    y           REAL    CHECK (y     BETWEEN 0 AND 100),  -- 胶质层厚度 mm
    petro       REAL    CHECK (petro >= 0),               -- 岩相
    csr         REAL    CHECK (csr   BETWEEN 0 AND 100),  -- 焦炭反应后强度 %
    m           REAL    CHECK (m     BETWEEN 0 AND 100),  -- 水分 %

    -- 价格 (元/吨)
    fob         REAL    CHECK (fob >= 0),                 -- 出厂价
    frt         REAL    CHECK (frt >= 0),                 -- 运费
    cif         REAL    GENERATED ALWAYS AS (fob + frt) VIRTUAL,  -- 到厂价 = 派生

    note        TEXT,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_mines_status ON mines(status);
CREATE INDEX IF NOT EXISTS idx_mines_region ON mines(province, city);

-- 每字段可信度 (稀疏: 只记录有可信度标注的字段)
CREATE TABLE IF NOT EXISTS mine_field_confidence (
    mine_id     INTEGER NOT NULL REFERENCES mines(id) ON DELETE CASCADE,
    field       TEXT    NOT NULL
                CHECK (field IN ('s','a','v','g','y','petro','csr','m','fob','frt')),
    confidence  TEXT    NOT NULL CHECK (confidence IN ('high','medium','low')),
    PRIMARY KEY (mine_id, field)
);

-- updated_at 自动维护
CREATE TRIGGER IF NOT EXISTS trg_mines_updated
AFTER UPDATE ON mines
BEGIN
    UPDATE mines SET updated_at = datetime('now') WHERE id = NEW.id;
END;

-- ============================================================
-- User 层: 用户私有数据 (override + 启用状态)
-- ============================================================
-- 用户对单个字段的 override.
-- master 升级时不会动这张表, 用户的改动得以保留.
CREATE TABLE IF NOT EXISTS user_overrides (
    coal_name    TEXT NOT NULL REFERENCES mines(name) ON DELETE CASCADE,
    field        TEXT NOT NULL,
    value        REAL NOT NULL,
    updated_at   TEXT NOT NULL,              -- ISO8601
    PRIMARY KEY (coal_name, field)
);

-- 用户对煤的偏好: 是否启用 / 今日价格 / 备注
CREATE TABLE IF NOT EXISTS user_coal_prefs (
    coal_name        TEXT PRIMARY KEY REFERENCES mines(name) ON DELETE CASCADE,
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
    contract_name   TEXT,                             -- 合同名快照 (web 端用名字, 不走 contract_id 外键)
    total_quantity  REAL,
    cost_cif        REAL NOT NULL,
    result_json     TEXT NOT NULL,                    -- 完整 BlendResult JSON, 混合后指标(回归 X)即在此
    csr_measured    REAL,                             -- 回填的实测焦炭 CSR (回归 y); NULL = 未回填
    note            TEXT
);

CREATE INDEX IF NOT EXISTS idx_history_occurred_at ON blend_history(occurred_at DESC);
"#;
