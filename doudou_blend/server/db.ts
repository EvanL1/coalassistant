/**
 * SQLite 数据库初始化 + schema 自启 bootstrap.
 *
 * 从 Cloudflare D1 迁过来. schema 跟 cloudflare/schema.sql 完全一致,
 * 之前 D1 上的 JSON 导出 (.dump) 可以直接 .read 进这个文件.
 *
 * 设计:
 *  - 生产: 单文件 SQLite, WAL 模式 (并发读 + 高性能), 路径走 DATA_DIR/data.db
 *  - 测试: in-memory (`:memory:`)
 */

import Database from "better-sqlite3";

export function openDatabase(path: string): Database.Database {
  const db = new Database(path);
  // WAL: 并发读, 写不阻塞读. 单用户场景没并发问题, 但加上没坏处.
  db.pragma("journal_mode = WAL");
  db.pragma("foreign_keys = ON");
  applySchema(db);
  return db;
}

export function openInMemory(): Database.Database {
  const db = new Database(":memory:");
  applySchema(db);
  return db;
}

/** 幂等建表. 改 schema 时改这里, 老数据库再次启动会自动补差异 (但只补 IF NOT EXISTS 的项, 改字段类型需要 migration). */
export function applySchema(db: Database.Database): void {
  db.exec(`
    CREATE TABLE IF NOT EXISTS user_coals (
      name TEXT PRIMARY KEY,
      region TEXT,
      coal_type TEXT,
      status TEXT NOT NULL DEFAULT 'draft',
      props_json TEXT NOT NULL DEFAULT '{}',
      fob REAL,
      frt REAL,
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_user_coals_status ON user_coals(status);
    CREATE INDEX IF NOT EXISTS idx_user_coals_updated ON user_coals(updated_at DESC);

    CREATE TABLE IF NOT EXISTS user_settings (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL,
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS customers (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      contact TEXT, phone TEXT, note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_customers_updated ON customers(updated_at DESC);
    CREATE INDEX IF NOT EXISTS idx_customers_name ON customers(name);

    CREATE TABLE IF NOT EXISTS quotes (
      id TEXT PRIMARY KEY,
      customer_id TEXT NOT NULL,
      customer_name TEXT NOT NULL,
      recipe_json TEXT NOT NULL,
      cost_cif REAL NOT NULL,
      markup REAL NOT NULL DEFAULT 0,
      quoted_price REAL NOT NULL,
      total_tons REAL,
      contract_name TEXT,
      status TEXT NOT NULL DEFAULT 'draft',
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_quotes_customer ON quotes(customer_id);
    CREATE INDEX IF NOT EXISTS idx_quotes_status ON quotes(status);
    CREATE INDEX IF NOT EXISTS idx_quotes_updated ON quotes(updated_at DESC);

    CREATE TABLE IF NOT EXISTS contracts (
      id TEXT PRIMARY KEY,
      quote_id TEXT,
      customer_id TEXT NOT NULL,
      customer_name TEXT NOT NULL,
      contract_no TEXT,
      billing_location TEXT,
      prepay_party TEXT,
      recipe_json TEXT NOT NULL,
      unit_price REAL NOT NULL,
      total_tons REAL NOT NULL,
      total_amount REAL NOT NULL,
      first_pay_pct REAL NOT NULL DEFAULT 80,
      first_pay_amount REAL NOT NULL,
      tail_pay_amount REAL NOT NULL,
      signed_at TEXT,
      status TEXT NOT NULL DEFAULT 'active',
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_contracts_customer ON contracts(customer_id);
    CREATE INDEX IF NOT EXISTS idx_contracts_status ON contracts(status);
    CREATE INDEX IF NOT EXISTS idx_contracts_updated ON contracts(updated_at DESC);

    CREATE TABLE IF NOT EXISTS payments (
      id TEXT PRIMARY KEY,
      contract_id TEXT NOT NULL,
      kind TEXT NOT NULL DEFAULT 'first',
      amount REAL NOT NULL,
      paid_at TEXT NOT NULL,
      payer TEXT, method TEXT, voucher_no TEXT, note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_payments_contract ON payments(contract_id);
    CREATE INDEX IF NOT EXISTS idx_payments_paid_at ON payments(paid_at DESC);

    CREATE TABLE IF NOT EXISTS shipments (
      id TEXT PRIMARY KEY,
      contract_id TEXT NOT NULL,
      vehicle_no TEXT,
      net_tons REAL NOT NULL,
      gross_tons REAL, tare_tons REAL,
      shipped_at TEXT NOT NULL,
      arrived_at TEXT, settled_at TEXT,
      settled_amount REAL,
      assay_json TEXT,
      status TEXT NOT NULL DEFAULT 'shipped',
      note TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_shipments_contract ON shipments(contract_id);
    CREATE INDEX IF NOT EXISTS idx_shipments_shipped ON shipments(shipped_at DESC);
  `);
}
