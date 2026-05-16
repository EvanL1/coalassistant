/**
 * Worker 入口 - 单文件统一处理 /api/* 路由 + 静态资源回退.
 *
 * Cloudflare Workers + Static Assets 模式:
 *   - 非 /api/* 请求 → env.ASSETS.fetch(req) 返回 dist/ 里的静态文件
 *   - /api/login            → 验密码返回 token (无需鉴权)
 *   - /api/coals (GET)      → 列表
 *   - /api/coals (POST)     → 新增单条
 *   - /api/coals/migrate    → 批量导入 (localStorage → D1 迁移)
 *   - /api/coals?name=X (DELETE) → 删除
 *
 * 单租户共享密码:
 *   - server secret AUTH_PASS (wrangler secret put)
 *   - 客户端登录拿到 token = AUTH_PASS, 后续请求 Authorization: Bearer <token>
 */

interface Env {
  ASSETS: Fetcher;
  DB: D1Database;
  AUTH_PASS: string;
}

interface MasterCoalEntry {
  name: string;
  region?: string | null;
  coal_type?: string | null;
  status: string;
  props: Record<string, number>;
  fob?: number | null;
  frt?: number | null;
  note?: string | null;
}

interface CoalRow {
  name: string;
  region: string | null;
  coal_type: string | null;
  status: string;
  props_json: string;
  fob: number | null;
  frt: number | null;
  note: string | null;
}

const AUTH_USER = "doudou";

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);

    if (url.pathname.startsWith("/api/")) {
      try {
        return await handleApi(req, env, url);
      } catch (e) {
        // 兜底: 任何未处理异常 (D1 错误 / 表缺失 / JSON 解析等) 都转成
        // 带 message 的 500, 否则 Cloudflare runtime 抛裸 500 客户端没法 debug.
        console.error("API error:", e);
        const msg = e instanceof Error ? e.message : String(e);
        return json({ ok: false, error: msg }, 500);
      }
    }

    // 静态资源 (前端 SPA) - Cloudflare ASSETS 已经处理 index.html fallback
    return env.ASSETS.fetch(req);
  },
};

// 每个 Worker isolate 启动后跑一次 CREATE TABLE IF NOT EXISTS, 失败则下次重试.
// 跟 cloudflare/schema.sql 保持一致 - 改 schema 两边都要改.
// 设计原因: phase1/2 每次加表都要手动 wrangler d1 execute, 容易忘 → 直接 500.
let schemaReady: Promise<void> | null = null;

async function ensureSchema(env: Env): Promise<void> {
  if (schemaReady) return schemaReady;
  schemaReady = (async () => {
    try {
      await env.DB.batch([
        env.DB.prepare(
          `CREATE TABLE IF NOT EXISTS user_coals (
             name TEXT PRIMARY KEY,
             region TEXT, coal_type TEXT,
             status TEXT NOT NULL DEFAULT 'draft',
             props_json TEXT NOT NULL DEFAULT '{}',
             fob REAL, frt REAL, note TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_user_coals_status ON user_coals(status)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_user_coals_updated ON user_coals(updated_at DESC)`,
        ),
        env.DB.prepare(
          `CREATE TABLE IF NOT EXISTS user_settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL,
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))`,
        ),
        env.DB.prepare(
          `CREATE TABLE IF NOT EXISTS customers (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             contact TEXT, phone TEXT, note TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_customers_updated ON customers(updated_at DESC)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_customers_name ON customers(name)`,
        ),
        env.DB.prepare(
          `CREATE TABLE IF NOT EXISTS quotes (
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
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_quotes_customer ON quotes(customer_id)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_quotes_status ON quotes(status)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_quotes_updated ON quotes(updated_at DESC)`,
        ),
        env.DB.prepare(
          `CREATE TABLE IF NOT EXISTS contracts (
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
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_contracts_customer ON contracts(customer_id)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_contracts_status ON contracts(status)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_contracts_updated ON contracts(updated_at DESC)`,
        ),
        env.DB.prepare(
          `CREATE TABLE IF NOT EXISTS payments (
             id TEXT PRIMARY KEY,
             contract_id TEXT NOT NULL,
             kind TEXT NOT NULL DEFAULT 'first',
             amount REAL NOT NULL,
             paid_at TEXT NOT NULL,
             payer TEXT, method TEXT, voucher_no TEXT, note TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')))`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_payments_contract ON payments(contract_id)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_payments_paid_at ON payments(paid_at DESC)`,
        ),
        env.DB.prepare(
          `CREATE TABLE IF NOT EXISTS shipments (
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
             created_at TEXT NOT NULL DEFAULT (datetime('now')))`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_shipments_contract ON shipments(contract_id)`,
        ),
        env.DB.prepare(
          `CREATE INDEX IF NOT EXISTS idx_shipments_shipped ON shipments(shipped_at DESC)`,
        ),
      ]);
    } catch (e) {
      schemaReady = null;
      throw e;
    }
  })();
  return schemaReady;
}

async function handleApi(req: Request, env: Env, url: URL): Promise<Response> {
  if (req.method === "OPTIONS") {
    return new Response(null, { status: 204, headers: corsHeaders() });
  }

  // /api/login 不需要鉴权 (不访 DB)
  if (url.pathname === "/api/login") {
    if (req.method !== "POST") return json({ error: "POST only" }, 405);
    return handleLogin(req, env);
  }

  // 其他路由统一 Bearer token 校验
  const auth = req.headers.get("Authorization") || "";
  const m = auth.match(/^Bearer\s+(.+)$/);
  if (!m || m[1] !== env.AUTH_PASS) {
    return json({ ok: false, error: "未授权" }, 401);
  }

  // 鉴权过了再 bootstrap schema, 避免未鉴权流量打 DB.
  await ensureSchema(env);

  if (url.pathname === "/api/coals/migrate") {
    if (req.method !== "POST") return json({ error: "POST only" }, 405);
    return handleMigrate(req, env);
  }

  if (url.pathname === "/api/coals") {
    if (req.method === "GET") return handleList(env);
    if (req.method === "POST") return handleAdd(req, env);
    if (req.method === "DELETE") return handleDelete(env, url);
    return json({ error: "Method not allowed" }, 405);
  }

  // 通用 KV: prefs / contract / history 都走这里
  if (url.pathname === "/api/settings") {
    if (req.method === "GET") return handleGetSetting(env, url);
    if (req.method === "PUT") return handlePutSetting(req, env);
    if (req.method === "DELETE") return handleDeleteSetting(env, url);
    return json({ error: "Method not allowed" }, 405);
  }

  if (url.pathname === "/api/customers") {
    if (req.method === "GET") return handleListCustomers(env);
    if (req.method === "POST") return handleUpsertCustomer(req, env);
    if (req.method === "DELETE") return handleDeleteCustomer(env, url);
    return json({ error: "Method not allowed" }, 405);
  }

  if (url.pathname === "/api/quotes") {
    if (req.method === "GET") return handleListQuotes(env);
    if (req.method === "POST") return handleUpsertQuote(req, env);
    if (req.method === "DELETE") return handleDeleteQuote(env, url);
    return json({ error: "Method not allowed" }, 405);
  }

  if (url.pathname === "/api/contracts") {
    if (req.method === "GET") return handleListContracts(env);
    if (req.method === "POST") return handleUpsertContract(req, env);
    if (req.method === "DELETE") return handleDeleteContract(env, url);
    return json({ error: "Method not allowed" }, 405);
  }

  if (url.pathname === "/api/payments") {
    if (req.method === "GET") return handleListPayments(env, url);
    if (req.method === "POST") return handleUpsertPayment(req, env);
    if (req.method === "DELETE") return handleDeletePayment(env, url);
    return json({ error: "Method not allowed" }, 405);
  }

  if (url.pathname === "/api/shipments") {
    if (req.method === "GET") return handleListShipments(env, url);
    if (req.method === "POST") return handleUpsertShipment(req, env);
    if (req.method === "DELETE") return handleDeleteShipment(env, url);
    return json({ error: "Method not allowed" }, 405);
  }

  return json({ error: "Not found" }, 404);
}

// ============================================================
// Handlers
// ============================================================

async function handleLogin(req: Request, env: Env): Promise<Response> {
  let body: { user?: string; pass?: string };
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  const user = (body.user ?? "").trim().toLowerCase();
  const pass = (body.pass ?? "").trim();

  if (user !== AUTH_USER) {
    return json({ ok: false, error: "账号或密码错误" }, 401);
  }
  if (!env.AUTH_PASS) {
    return json(
      { ok: false, error: "服务器未配置 AUTH_PASS, 联系管理员" },
      500,
    );
  }
  if (pass !== env.AUTH_PASS) {
    return json({ ok: false, error: "账号或密码错误" }, 401);
  }
  return json({ ok: true, token: env.AUTH_PASS });
}

async function handleList(env: Env): Promise<Response> {
  const rows = await env.DB.prepare(
    `SELECT name, region, coal_type, status, props_json, fob, frt, note
       FROM user_coals
       ORDER BY updated_at DESC`,
  ).all<CoalRow>();
  const coals: MasterCoalEntry[] = (rows.results ?? []).map(rowToEntry);
  return json({ coals });
}

async function handleAdd(req: Request, env: Env): Promise<Response> {
  let coal: MasterCoalEntry;
  try {
    coal = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  if (!coal.name || !coal.name.trim()) {
    return json({ ok: false, error: "煤名不能为空" }, 400);
  }
  try {
    await env.DB.prepare(
      `INSERT INTO user_coals
         (name, region, coal_type, status, props_json, fob, frt, note)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
    )
      .bind(
        coal.name.trim(),
        coal.region ?? null,
        coal.coal_type ?? null,
        coal.status || "draft",
        JSON.stringify(coal.props ?? {}),
        coal.fob ?? null,
        coal.frt ?? null,
        coal.note ?? null,
      )
      .run();
    return json({ ok: true });
  } catch (e) {
    // 仅把 UNIQUE 冲突映射成 409, 其他错误 (表缺失 / 类型不匹配等) 让 fetch
    // 层的 try/catch 兜底成 500 并带真实信息, 避免误把"煤种已存在"挡在前面.
    const msg = e instanceof Error ? e.message : String(e);
    if (/UNIQUE|PRIMARY KEY/i.test(msg)) {
      return json({ ok: false, error: "煤种已存在" }, 409);
    }
    throw e;
  }
}

async function handleDelete(env: Env, url: URL): Promise<Response> {
  const name = url.searchParams.get("name");
  if (!name) return json({ ok: false, error: "缺少 name" }, 400);
  await env.DB.prepare(`DELETE FROM user_coals WHERE name = ?`)
    .bind(name)
    .run();
  return json({ ok: true });
}

async function handleMigrate(req: Request, env: Env): Promise<Response> {
  let body: { coals?: MasterCoalEntry[] };
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  const list = body.coals ?? [];
  let imported = 0;
  for (const c of list) {
    try {
      await env.DB.prepare(
        `INSERT INTO user_coals
           (name, region, coal_type, status, props_json, fob, frt, note)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET
           region     = excluded.region,
           coal_type  = excluded.coal_type,
           status     = excluded.status,
           props_json = excluded.props_json,
           fob        = excluded.fob,
           frt        = excluded.frt,
           note       = excluded.note,
           updated_at = datetime('now')`,
      )
        .bind(
          c.name.trim(),
          c.region ?? null,
          c.coal_type ?? null,
          c.status || "draft",
          JSON.stringify(c.props ?? {}),
          c.fob ?? null,
          c.frt ?? null,
          c.note ?? null,
        )
        .run();
      imported += 1;
    } catch {
      // 单条失败不影响其他
    }
  }
  return json({ ok: true, imported });
}

// ============================================================
// Settings KV (coal_prefs / user_contract / history)
// ============================================================

async function handleGetSetting(env: Env, url: URL): Promise<Response> {
  const key = url.searchParams.get("key");
  if (!key) return json({ ok: false, error: "缺少 key" }, 400);
  const row = await env.DB.prepare(
    `SELECT value FROM user_settings WHERE key = ?`,
  )
    .bind(key)
    .first<{ value: string }>();
  return json({ value: row?.value ?? null });
}

async function handlePutSetting(req: Request, env: Env): Promise<Response> {
  let body: { key?: string; value?: string };
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  const { key, value } = body;
  if (!key || typeof value !== "string") {
    return json({ ok: false, error: "缺少 key/value" }, 400);
  }
  await env.DB.prepare(
    `INSERT INTO user_settings (key, value)
     VALUES (?, ?)
     ON CONFLICT(key) DO UPDATE SET
       value = excluded.value,
       updated_at = datetime('now')`,
  )
    .bind(key, value)
    .run();
  return json({ ok: true });
}

async function handleDeleteSetting(env: Env, url: URL): Promise<Response> {
  const key = url.searchParams.get("key");
  if (!key) return json({ ok: false, error: "缺少 key" }, 400);
  await env.DB.prepare(`DELETE FROM user_settings WHERE key = ?`)
    .bind(key)
    .run();
  return json({ ok: true });
}

// ============================================================
// Customers
// ============================================================

interface CustomerRow {
  id: string;
  name: string;
  contact: string | null;
  phone: string | null;
  note: string | null;
  created_at: string;
  updated_at: string;
}

async function handleListCustomers(env: Env): Promise<Response> {
  const rows = await env.DB.prepare(
    `SELECT id, name, contact, phone, note, created_at, updated_at
       FROM customers ORDER BY updated_at DESC`,
  ).all<CustomerRow>();
  return json({ customers: rows.results ?? [] });
}

async function handleUpsertCustomer(req: Request, env: Env): Promise<Response> {
  let body: Partial<CustomerRow>;
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  if (!body.id || !body.name?.trim()) {
    return json({ ok: false, error: "缺少 id 或 name" }, 400);
  }
  await env.DB.prepare(
    `INSERT INTO customers (id, name, contact, phone, note)
     VALUES (?, ?, ?, ?, ?)
     ON CONFLICT(id) DO UPDATE SET
       name = excluded.name,
       contact = excluded.contact,
       phone = excluded.phone,
       note = excluded.note,
       updated_at = datetime('now')`,
  )
    .bind(
      body.id,
      body.name.trim(),
      body.contact ?? null,
      body.phone ?? null,
      body.note ?? null,
    )
    .run();
  return json({ ok: true });
}

async function handleDeleteCustomer(env: Env, url: URL): Promise<Response> {
  const id = url.searchParams.get("id");
  if (!id) return json({ ok: false, error: "缺少 id" }, 400);
  await env.DB.prepare(`DELETE FROM customers WHERE id = ?`).bind(id).run();
  return json({ ok: true });
}

// ============================================================
// Quotes
// ============================================================

interface QuoteRow {
  id: string;
  customer_id: string;
  customer_name: string;
  recipe_json: string;
  cost_cif: number;
  markup: number;
  quoted_price: number;
  total_tons: number | null;
  contract_name: string | null;
  status: string;
  note: string | null;
  created_at: string;
  updated_at: string;
}

async function handleListQuotes(env: Env): Promise<Response> {
  const rows = await env.DB.prepare(
    `SELECT id, customer_id, customer_name, recipe_json, cost_cif, markup,
            quoted_price, total_tons, contract_name, status, note,
            created_at, updated_at
       FROM quotes ORDER BY updated_at DESC`,
  ).all<QuoteRow>();
  return json({ quotes: rows.results ?? [] });
}

async function handleUpsertQuote(req: Request, env: Env): Promise<Response> {
  let body: Partial<QuoteRow>;
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  if (!body.id || !body.customer_id || !body.customer_name) {
    return json({ ok: false, error: "缺少必填字段" }, 400);
  }
  await env.DB.prepare(
    `INSERT INTO quotes
       (id, customer_id, customer_name, recipe_json, cost_cif, markup,
        quoted_price, total_tons, contract_name, status, note)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT(id) DO UPDATE SET
       customer_id   = excluded.customer_id,
       customer_name = excluded.customer_name,
       recipe_json   = excluded.recipe_json,
       cost_cif      = excluded.cost_cif,
       markup        = excluded.markup,
       quoted_price  = excluded.quoted_price,
       total_tons    = excluded.total_tons,
       contract_name = excluded.contract_name,
       status        = excluded.status,
       note          = excluded.note,
       updated_at    = datetime('now')`,
  )
    .bind(
      body.id,
      body.customer_id,
      body.customer_name,
      body.recipe_json ?? "{}",
      body.cost_cif ?? 0,
      body.markup ?? 0,
      body.quoted_price ?? 0,
      body.total_tons ?? null,
      body.contract_name ?? null,
      body.status ?? "draft",
      body.note ?? null,
    )
    .run();
  return json({ ok: true });
}

async function handleDeleteQuote(env: Env, url: URL): Promise<Response> {
  const id = url.searchParams.get("id");
  if (!id) return json({ ok: false, error: "缺少 id" }, 400);
  await env.DB.prepare(`DELETE FROM quotes WHERE id = ?`).bind(id).run();
  return json({ ok: true });
}

// ============================================================
// Contracts
// ============================================================

interface ContractRow {
  id: string;
  quote_id: string | null;
  customer_id: string;
  customer_name: string;
  contract_no: string | null;
  billing_location: string | null;
  prepay_party: string | null;
  recipe_json: string;
  unit_price: number;
  total_tons: number;
  total_amount: number;
  first_pay_pct: number;
  first_pay_amount: number;
  tail_pay_amount: number;
  signed_at: string | null;
  status: string;
  note: string | null;
  created_at: string;
  updated_at: string;
}

async function handleListContracts(env: Env): Promise<Response> {
  const rows = await env.DB.prepare(
    `SELECT id, quote_id, customer_id, customer_name, contract_no,
            billing_location, prepay_party, recipe_json, unit_price,
            total_tons, total_amount, first_pay_pct, first_pay_amount,
            tail_pay_amount, signed_at, status, note,
            created_at, updated_at
       FROM contracts ORDER BY updated_at DESC`,
  ).all<ContractRow>();
  return json({ contracts: rows.results ?? [] });
}

async function handleUpsertContract(req: Request, env: Env): Promise<Response> {
  let body: Partial<ContractRow>;
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  if (!body.id || !body.customer_id || !body.customer_name) {
    return json({ ok: false, error: "缺少必填字段" }, 400);
  }
  await env.DB.prepare(
    `INSERT INTO contracts
       (id, quote_id, customer_id, customer_name, contract_no,
        billing_location, prepay_party, recipe_json, unit_price,
        total_tons, total_amount, first_pay_pct, first_pay_amount,
        tail_pay_amount, signed_at, status, note)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT(id) DO UPDATE SET
       contract_no      = excluded.contract_no,
       billing_location = excluded.billing_location,
       prepay_party     = excluded.prepay_party,
       unit_price       = excluded.unit_price,
       total_tons       = excluded.total_tons,
       total_amount     = excluded.total_amount,
       first_pay_pct    = excluded.first_pay_pct,
       first_pay_amount = excluded.first_pay_amount,
       tail_pay_amount  = excluded.tail_pay_amount,
       signed_at        = excluded.signed_at,
       status           = excluded.status,
       note             = excluded.note,
       updated_at       = datetime('now')`,
  )
    .bind(
      body.id,
      body.quote_id ?? null,
      body.customer_id,
      body.customer_name,
      body.contract_no ?? null,
      body.billing_location ?? null,
      body.prepay_party ?? null,
      body.recipe_json ?? "{}",
      body.unit_price ?? 0,
      body.total_tons ?? 0,
      body.total_amount ?? 0,
      body.first_pay_pct ?? 80,
      body.first_pay_amount ?? 0,
      body.tail_pay_amount ?? 0,
      body.signed_at ?? null,
      body.status ?? "active",
      body.note ?? null,
    )
    .run();
  return json({ ok: true });
}

async function handleDeleteContract(env: Env, url: URL): Promise<Response> {
  const id = url.searchParams.get("id");
  if (!id) return json({ ok: false, error: "缺少 id" }, 400);
  // 级联删除 (D1 默认不开 foreign key cascade, 手动)
  await env.DB.batch([
    env.DB.prepare(`DELETE FROM payments WHERE contract_id = ?`).bind(id),
    env.DB.prepare(`DELETE FROM shipments WHERE contract_id = ?`).bind(id),
    env.DB.prepare(`DELETE FROM contracts WHERE id = ?`).bind(id),
  ]);
  return json({ ok: true });
}

// ============================================================
// Payments
// ============================================================

interface PaymentRow {
  id: string;
  contract_id: string;
  kind: string;
  amount: number;
  paid_at: string;
  payer: string | null;
  method: string | null;
  voucher_no: string | null;
  note: string | null;
  created_at: string;
}

async function handleListPayments(env: Env, url: URL): Promise<Response> {
  const contractId = url.searchParams.get("contract_id");
  let rows;
  if (contractId) {
    rows = await env.DB.prepare(
      `SELECT id, contract_id, kind, amount, paid_at, payer, method,
              voucher_no, note, created_at
         FROM payments WHERE contract_id = ? ORDER BY paid_at DESC`,
    )
      .bind(contractId)
      .all<PaymentRow>();
  } else {
    rows = await env.DB.prepare(
      `SELECT id, contract_id, kind, amount, paid_at, payer, method,
              voucher_no, note, created_at
         FROM payments ORDER BY paid_at DESC`,
    ).all<PaymentRow>();
  }
  return json({ payments: rows.results ?? [] });
}

async function handleUpsertPayment(req: Request, env: Env): Promise<Response> {
  let body: Partial<PaymentRow>;
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  if (!body.id || !body.contract_id || body.amount == null || !body.paid_at) {
    return json({ ok: false, error: "缺少必填字段" }, 400);
  }
  await env.DB.prepare(
    `INSERT INTO payments
       (id, contract_id, kind, amount, paid_at, payer, method, voucher_no, note)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT(id) DO UPDATE SET
       kind       = excluded.kind,
       amount     = excluded.amount,
       paid_at    = excluded.paid_at,
       payer      = excluded.payer,
       method     = excluded.method,
       voucher_no = excluded.voucher_no,
       note       = excluded.note`,
  )
    .bind(
      body.id,
      body.contract_id,
      body.kind ?? "first",
      body.amount,
      body.paid_at,
      body.payer ?? null,
      body.method ?? null,
      body.voucher_no ?? null,
      body.note ?? null,
    )
    .run();
  return json({ ok: true });
}

async function handleDeletePayment(env: Env, url: URL): Promise<Response> {
  const id = url.searchParams.get("id");
  if (!id) return json({ ok: false, error: "缺少 id" }, 400);
  await env.DB.prepare(`DELETE FROM payments WHERE id = ?`).bind(id).run();
  return json({ ok: true });
}

// ============================================================
// Shipments
// ============================================================

interface ShipmentRow {
  id: string;
  contract_id: string;
  vehicle_no: string | null;
  net_tons: number;
  gross_tons: number | null;
  tare_tons: number | null;
  shipped_at: string;
  arrived_at: string | null;
  settled_at: string | null;
  settled_amount: number | null;
  assay_json: string | null;
  status: string;
  note: string | null;
  created_at: string;
}

async function handleListShipments(env: Env, url: URL): Promise<Response> {
  const contractId = url.searchParams.get("contract_id");
  let rows;
  if (contractId) {
    rows = await env.DB.prepare(
      `SELECT id, contract_id, vehicle_no, net_tons, gross_tons, tare_tons,
              shipped_at, arrived_at, settled_at, settled_amount,
              assay_json, status, note, created_at
         FROM shipments WHERE contract_id = ? ORDER BY shipped_at DESC`,
    )
      .bind(contractId)
      .all<ShipmentRow>();
  } else {
    rows = await env.DB.prepare(
      `SELECT id, contract_id, vehicle_no, net_tons, gross_tons, tare_tons,
              shipped_at, arrived_at, settled_at, settled_amount,
              assay_json, status, note, created_at
         FROM shipments ORDER BY shipped_at DESC`,
    ).all<ShipmentRow>();
  }
  return json({ shipments: rows.results ?? [] });
}

async function handleUpsertShipment(req: Request, env: Env): Promise<Response> {
  let body: Partial<ShipmentRow>;
  try {
    body = await req.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  if (!body.id || !body.contract_id || body.net_tons == null || !body.shipped_at) {
    return json({ ok: false, error: "缺少必填字段" }, 400);
  }
  await env.DB.prepare(
    `INSERT INTO shipments
       (id, contract_id, vehicle_no, net_tons, gross_tons, tare_tons,
        shipped_at, arrived_at, settled_at, settled_amount,
        assay_json, status, note)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT(id) DO UPDATE SET
       vehicle_no     = excluded.vehicle_no,
       net_tons       = excluded.net_tons,
       gross_tons     = excluded.gross_tons,
       tare_tons      = excluded.tare_tons,
       shipped_at     = excluded.shipped_at,
       arrived_at     = excluded.arrived_at,
       settled_at     = excluded.settled_at,
       settled_amount = excluded.settled_amount,
       assay_json     = excluded.assay_json,
       status         = excluded.status,
       note           = excluded.note`,
  )
    .bind(
      body.id,
      body.contract_id,
      body.vehicle_no ?? null,
      body.net_tons,
      body.gross_tons ?? null,
      body.tare_tons ?? null,
      body.shipped_at,
      body.arrived_at ?? null,
      body.settled_at ?? null,
      body.settled_amount ?? null,
      body.assay_json ?? null,
      body.status ?? "shipped",
      body.note ?? null,
    )
    .run();
  return json({ ok: true });
}

async function handleDeleteShipment(env: Env, url: URL): Promise<Response> {
  const id = url.searchParams.get("id");
  if (!id) return json({ ok: false, error: "缺少 id" }, 400);
  await env.DB.prepare(`DELETE FROM shipments WHERE id = ?`).bind(id).run();
  return json({ ok: true });
}

// ============================================================
// Utils
// ============================================================

function rowToEntry(r: CoalRow): MasterCoalEntry {
  let props: Record<string, number> = {};
  try {
    props = JSON.parse(r.props_json) as Record<string, number>;
  } catch {
    props = {};
  }
  return {
    name: r.name,
    region: r.region,
    coal_type: r.coal_type,
    status: r.status,
    props,
    fob: r.fob,
    frt: r.frt,
    note: r.note,
  };
}

function corsHeaders(): Record<string, string> {
  return {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, DELETE, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization",
    "Access-Control-Max-Age": "86400",
  };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      ...corsHeaders(),
    },
  });
}
