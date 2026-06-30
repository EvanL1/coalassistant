-- 豆哥配煤 · 煤源(矿源)数据库结构
-- 目标: 严格、可扩展到 1000+ 座煤矿。一行一个煤源。
-- 约定:
--   * 指标缺测一律 NULL,不用占位值(占位 = 把"没测"伪装成"测了")
--   * 派生量(CIF)用生成列,不手存
--   * 文字地址用于定位;经纬度可空,后续 geocode 自动补
-- SQLite 方言(项目用 rusqlite + bundled SQLite)。

PRAGMA foreign_keys = ON;

-- ============================================================
-- 煤源主表
-- ============================================================
CREATE TABLE mines (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,                  -- 煤名(唯一键)
    coal_type   TEXT,                                     -- 煤种,如 "主焦煤"
    status      TEXT    NOT NULL DEFAULT 'incomplete'
                CHECK (status IN ('verified','active','draft','incomplete','archived')),

    -- 位置
    province    TEXT,                                     -- 省
    city        TEXT,                                     -- 市
    county      TEXT,                                     -- 县/区
    mine_name   TEXT,                                     -- 矿点名
    lat         REAL    CHECK (lat BETWEEN -90  AND 90),  -- 纬度(可空,后补)
    lng         REAL    CHECK (lng BETWEEN -180 AND 180), -- 经度(可空,后补)

    -- 煤质指标(8 项,缺测为 NULL)
    s           REAL    CHECK (s     BETWEEN 0 AND 100),  -- 硫 %
    a           REAL    CHECK (a     BETWEEN 0 AND 100),  -- 灰 %
    v           REAL    CHECK (v     BETWEEN 0 AND 100),  -- 挥发 %
    g           REAL    CHECK (g     BETWEEN 0 AND 100),  -- 粘结指数 G
    y           REAL    CHECK (y     BETWEEN 0 AND 100),  -- 胶质层厚度 mm
    petro       REAL    CHECK (petro >= 0),               -- 岩相(反射率分布度量)
    csr         REAL    CHECK (csr   BETWEEN 0 AND 100),  -- 焦炭反应后强度 %
    m           REAL    CHECK (m     BETWEEN 0 AND 100),  -- 水分 %

    -- 价格(元/吨)
    fob         REAL    CHECK (fob >= 0),                 -- 出厂价
    frt         REAL    CHECK (frt >= 0),                 -- 运费
    cif         REAL    GENERATED ALWAYS AS (fob + frt) VIRTUAL,  -- 到厂价 = 派生,不手存

    note        TEXT,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_mines_status ON mines(status);
CREATE INDEX idx_mines_region ON mines(province, city);

-- ============================================================
-- 每字段可信度(稀疏:只记录有可信度标注的字段)
--   与主表分开,避免主表再多 10 列;一行 = 某座矿某个字段的可信度。
-- ============================================================
CREATE TABLE mine_field_confidence (
    mine_id     INTEGER NOT NULL REFERENCES mines(id) ON DELETE CASCADE,
    field       TEXT    NOT NULL
                CHECK (field IN ('s','a','v','g','y','petro','csr','m','fob','frt')),
    confidence  TEXT    NOT NULL
                CHECK (confidence IN ('high','medium','low')),
    PRIMARY KEY (mine_id, field)
);

-- ============================================================
-- updated_at 自动维护
-- ============================================================
CREATE TRIGGER trg_mines_updated
AFTER UPDATE ON mines
BEGIN
    UPDATE mines SET updated_at = datetime('now') WHERE id = NEW.id;
END;
