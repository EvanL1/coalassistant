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
