-- Cloudflare D1 schema: 用户新增的煤种 (单租户共享库)
--
-- 部署:
--   wrangler d1 execute coalassistant --remote --file=cloudflare/schema.sql
--
-- 本地开发:
--   wrangler d1 execute coalassistant --local --file=cloudflare/schema.sql

CREATE TABLE IF NOT EXISTS user_coals (
  name        TEXT    PRIMARY KEY,
  region      TEXT,
  coal_type   TEXT,
  status      TEXT    NOT NULL DEFAULT 'draft',
  props_json  TEXT    NOT NULL DEFAULT '{}',
  fob         REAL,
  frt         REAL,
  note        TEXT,
  created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
  updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_user_coals_status ON user_coals(status);
CREATE INDEX IF NOT EXISTS idx_user_coals_updated ON user_coals(updated_at DESC);

-- ============================================================
-- user_settings: 通用 KV, 存所有非煤的用户数据 (单租户共享)
-- ============================================================
-- 当前 key 约定:
--   coal_prefs       value = JSON dict, key=煤名, value=CoalPref
--   user_contract    value = JSON Spec[], 用户改过的合同 (null/不存在 = 用 master 默认)
--   history          value = JSON HistoryEntry[] (最近 100 条, 整组覆盖写)
--
-- 设计取舍: 简化为 KV 而不是为每种数据开表, 因为:
--   - 数据量小 (几十条 prefs, 1 个 contract, 100 条 history)
--   - 多设备并发写概率极低 (单租户两口子用)
--   - 整组写覆盖丢数据的代价可接受
--
-- 升级路径: 真要细粒度并发安全 (多用户/团队), 拆分为
--   coal_prefs / user_contracts / history 三张表.

CREATE TABLE IF NOT EXISTS user_settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
