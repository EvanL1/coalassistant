/**
 * Hono app for the Node.js / better-sqlite3 port of the Cloudflare Worker.
 *
 * Mirrors worker/index.ts 1:1: same routes, same JSON shapes, same status codes,
 * same field defaults. Schema bootstrap lives in db.ts (caller's responsibility).
 *
 * Auth: Authorization: Bearer <AUTH_PASS>. /api/login is exempt.
 */

import { Hono } from 'hono';
import type { Context, MiddlewareHandler } from 'hono';
import type Database from 'better-sqlite3';

export interface AppDeps {
  db: Database.Database;
  authPass: string;
}

const AUTH_USER = 'doudou';

// ============================================================
// Row / payload types (kept aligned with worker/index.ts)
// ============================================================

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

interface CustomerRow {
  id: string;
  name: string;
  contact: string | null;
  phone: string | null;
  note: string | null;
  created_at: string;
  updated_at: string;
}

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

// ============================================================
// Helpers
// ============================================================

const CORS_HEADERS: Record<string, string> = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
  'Access-Control-Max-Age': '86400',
};

/** Parse JSON body or return null + error response. Mirrors worker's
 *  "请求体不是合法 JSON" → 400 contract (test expects error to contain "JSON"). */
async function readJson<T>(c: Context): Promise<{ body: T } | { error: Response }> {
  try {
    const body = (await c.req.json()) as T;
    return { body };
  } catch {
    return {
      error: c.json({ ok: false, error: '请求体不是合法 JSON' }, 400),
    };
  }
}

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

// ============================================================
// App
// ============================================================

export function createApp(deps: AppDeps): Hono {
  const app = new Hono();
  const { db, authPass } = deps;

  // ---- CORS (apply CORS headers on every response, incl. errors) ----
  app.use('*', async (c, next) => {
    if (c.req.method === 'OPTIONS' && c.req.path.startsWith('/api/')) {
      return new Response(null, { status: 204, headers: CORS_HEADERS });
    }
    await next();
    for (const [k, v] of Object.entries(CORS_HEADERS)) {
      c.res.headers.set(k, v);
    }
  });

  // ---- Auth middleware (everything under /api/* except /api/login) ----
  const authMiddleware: MiddlewareHandler = async (c, next) => {
    if (c.req.path === '/api/login') return next();
    const auth = c.req.header('Authorization') ?? '';
    const m = auth.match(/^Bearer\s+(.+)$/);
    if (!m || m[1] !== authPass) {
      return c.json({ ok: false, error: '未授权' }, 401);
    }
    await next();
  };
  app.use('/api/*', authMiddleware);

  // ---- Top-level error handler ----
  app.onError((err, c) => {
    console.error('API error:', err);
    const msg = err instanceof Error ? err.message : String(err);
    return c.json({ ok: false, error: msg }, 500);
  });

  // ============================================================
  // /api/login
  // ============================================================
  app.post('/api/login', async (c) => {
    const parsed = await readJson<{ user?: string; pass?: string }>(c);
    if ('error' in parsed) return parsed.error;
    const body = parsed.body;
    const user = (body.user ?? '').trim().toLowerCase();
    const pass = (body.pass ?? '').trim();

    if (user !== AUTH_USER) {
      return c.json({ ok: false, error: '账号或密码错误' }, 401);
    }
    if (!authPass) {
      return c.json(
        { ok: false, error: '服务器未配置 AUTH_PASS, 联系管理员' },
        500,
      );
    }
    if (pass !== authPass) {
      return c.json({ ok: false, error: '账号或密码错误' }, 401);
    }
    return c.json({ ok: true, token: authPass });
  });

  // Reject non-POST on /api/login → 405 (matches worker)
  app.all('/api/login', (c) => c.json({ error: 'POST only' }, 405));

  // ============================================================
  // /api/coals
  // ============================================================
  app.get('/api/coals', (c) => {
    const rows = db
      .prepare(
        `SELECT name, region, coal_type, status, props_json, fob, frt, note
           FROM user_coals
           ORDER BY updated_at DESC`,
      )
      .all() as CoalRow[];
    const coals: MasterCoalEntry[] = rows.map(rowToEntry);
    return c.json({ coals });
  });

  app.post('/api/coals', async (c) => {
    const parsed = await readJson<MasterCoalEntry>(c);
    if ('error' in parsed) return parsed.error;
    const coal = parsed.body;
    if (!coal.name || !coal.name.trim()) {
      return c.json({ ok: false, error: '煤名不能为空' }, 400);
    }
    try {
      db.prepare(
        `INSERT INTO user_coals
           (name, region, coal_type, status, props_json, fob, frt, note)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        coal.name.trim(),
        coal.region ?? null,
        coal.coal_type ?? null,
        coal.status || 'draft',
        JSON.stringify(coal.props ?? {}),
        coal.fob ?? null,
        coal.frt ?? null,
        coal.note ?? null,
      );
      return c.json({ ok: true });
    } catch (e) {
      // Only map UNIQUE/PRIMARY KEY conflict to 409 — other errors bubble to onError.
      // better-sqlite3 throws SqliteError with code SQLITE_CONSTRAINT_PRIMARYKEY
      // (or _UNIQUE). Match on code AND message for safety.
      const msg = e instanceof Error ? e.message : String(e);
      const code = (e as { code?: string } | null)?.code ?? '';
      if (
        /UNIQUE|PRIMARY KEY/i.test(msg) ||
        code === 'SQLITE_CONSTRAINT_PRIMARYKEY' ||
        code === 'SQLITE_CONSTRAINT_UNIQUE'
      ) {
        return c.json({ ok: false, error: '煤种已存在' }, 409);
      }
      throw e;
    }
  });

  app.delete('/api/coals', (c) => {
    const name = c.req.query('name');
    if (!name) return c.json({ ok: false, error: '缺少 name' }, 400);
    db.prepare(`DELETE FROM user_coals WHERE name = ?`).run(name);
    return c.json({ ok: true });
  });

  // Catch-all for unsupported methods on /api/coals
  app.all('/api/coals', (c) => c.json({ error: 'Method not allowed' }, 405));

  // ============================================================
  // /api/coals/migrate
  // ============================================================
  app.post('/api/coals/migrate', async (c) => {
    const parsed = await readJson<{ coals?: MasterCoalEntry[] }>(c);
    if ('error' in parsed) return parsed.error;
    const list = parsed.body.coals ?? [];

    const stmt = db.prepare(
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
    );

    let imported = 0;
    for (const coal of list) {
      try {
        stmt.run(
          coal.name.trim(),
          coal.region ?? null,
          coal.coal_type ?? null,
          coal.status || 'draft',
          JSON.stringify(coal.props ?? {}),
          coal.fob ?? null,
          coal.frt ?? null,
          coal.note ?? null,
        );
        imported += 1;
      } catch {
        // Single row failure should not abort the batch (mirrors worker).
      }
    }
    return c.json({ ok: true, imported });
  });

  app.all('/api/coals/migrate', (c) => c.json({ error: 'POST only' }, 405));

  // ============================================================
  // /api/settings  (KV: coal_prefs / user_contract / history)
  // Worker uses PUT for upsert (tests confirm). Keep PUT.
  // ============================================================
  app.get('/api/settings', (c) => {
    const key = c.req.query('key');
    if (!key) return c.json({ ok: false, error: '缺少 key' }, 400);
    const row = db
      .prepare(`SELECT value FROM user_settings WHERE key = ?`)
      .get(key) as { value: string } | undefined;
    return c.json({ value: row?.value ?? null });
  });

  // Settings upsert: worker uses PUT (test contract). Spec also mentions POST —
  // accept both so any future front-end migration "just works".
  const putSetting = async (c: Context) => {
    const parsed = await readJson<{ key?: string; value?: string }>(c);
    if ('error' in parsed) return parsed.error;
    const { key, value } = parsed.body;
    // typeof check: empty string '' is a valid value (regression locked by test).
    if (!key || typeof value !== 'string') {
      return c.json({ ok: false, error: '缺少 key/value' }, 400);
    }
    db.prepare(
      `INSERT INTO user_settings (key, value)
       VALUES (?, ?)
       ON CONFLICT(key) DO UPDATE SET
         value      = excluded.value,
         updated_at = datetime('now')`,
    ).run(key, value);
    return c.json({ ok: true });
  };
  app.put('/api/settings', putSetting);
  app.post('/api/settings', putSetting);

  app.delete('/api/settings', (c) => {
    const key = c.req.query('key');
    if (!key) return c.json({ ok: false, error: '缺少 key' }, 400);
    db.prepare(`DELETE FROM user_settings WHERE key = ?`).run(key);
    return c.json({ ok: true });
  });

  app.all('/api/settings', (c) => c.json({ error: 'Method not allowed' }, 405));

  // ============================================================
  // /api/customers
  // ============================================================
  app.get('/api/customers', (c) => {
    const rows = db
      .prepare(
        `SELECT id, name, contact, phone, note, created_at, updated_at
           FROM customers ORDER BY updated_at DESC`,
      )
      .all() as CustomerRow[];
    return c.json({ customers: rows });
  });

  app.post('/api/customers', async (c) => {
    const parsed = await readJson<Partial<CustomerRow>>(c);
    if ('error' in parsed) return parsed.error;
    const body = parsed.body;
    if (!body.id || !body.name?.trim()) {
      return c.json({ ok: false, error: '缺少 id 或 name' }, 400);
    }
    db.prepare(
      `INSERT INTO customers (id, name, contact, phone, note)
       VALUES (?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
         name       = excluded.name,
         contact    = excluded.contact,
         phone      = excluded.phone,
         note       = excluded.note,
         updated_at = datetime('now')`,
    ).run(
      body.id,
      body.name.trim(),
      body.contact ?? null,
      body.phone ?? null,
      body.note ?? null,
    );
    return c.json({ ok: true });
  });

  app.delete('/api/customers', (c) => {
    const id = c.req.query('id');
    if (!id) return c.json({ ok: false, error: '缺少 id' }, 400);
    db.prepare(`DELETE FROM customers WHERE id = ?`).run(id);
    return c.json({ ok: true });
  });

  app.all('/api/customers', (c) => c.json({ error: 'Method not allowed' }, 405));

  // ============================================================
  // /api/quotes
  // ============================================================
  app.get('/api/quotes', (c) => {
    const rows = db
      .prepare(
        `SELECT id, customer_id, customer_name, recipe_json, cost_cif, markup,
                quoted_price, total_tons, contract_name, status, note,
                created_at, updated_at
           FROM quotes ORDER BY updated_at DESC`,
      )
      .all() as QuoteRow[];
    return c.json({ quotes: rows });
  });

  app.post('/api/quotes', async (c) => {
    const parsed = await readJson<Partial<QuoteRow>>(c);
    if ('error' in parsed) return parsed.error;
    const body = parsed.body;
    if (!body.id || !body.customer_id || !body.customer_name) {
      return c.json({ ok: false, error: '缺少必填字段' }, 400);
    }
    db.prepare(
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
    ).run(
      body.id,
      body.customer_id,
      body.customer_name,
      body.recipe_json ?? '{}',
      body.cost_cif ?? 0,
      body.markup ?? 0,
      body.quoted_price ?? 0,
      body.total_tons ?? null,
      body.contract_name ?? null,
      body.status ?? 'draft',
      body.note ?? null,
    );
    return c.json({ ok: true });
  });

  app.delete('/api/quotes', (c) => {
    const id = c.req.query('id');
    if (!id) return c.json({ ok: false, error: '缺少 id' }, 400);
    db.prepare(`DELETE FROM quotes WHERE id = ?`).run(id);
    return c.json({ ok: true });
  });

  app.all('/api/quotes', (c) => c.json({ error: 'Method not allowed' }, 405));

  // ============================================================
  // /api/contracts
  // ============================================================
  app.get('/api/contracts', (c) => {
    const rows = db
      .prepare(
        `SELECT id, quote_id, customer_id, customer_name, contract_no,
                billing_location, prepay_party, recipe_json, unit_price,
                total_tons, total_amount, first_pay_pct, first_pay_amount,
                tail_pay_amount, signed_at, status, note,
                created_at, updated_at
           FROM contracts ORDER BY updated_at DESC`,
      )
      .all() as ContractRow[];
    return c.json({ contracts: rows });
  });

  app.post('/api/contracts', async (c) => {
    const parsed = await readJson<Partial<ContractRow>>(c);
    if ('error' in parsed) return parsed.error;
    const body = parsed.body;
    if (!body.id || !body.customer_id || !body.customer_name) {
      return c.json({ ok: false, error: '缺少必填字段' }, 400);
    }
    db.prepare(
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
    ).run(
      body.id,
      body.quote_id ?? null,
      body.customer_id,
      body.customer_name,
      body.contract_no ?? null,
      body.billing_location ?? null,
      body.prepay_party ?? null,
      body.recipe_json ?? '{}',
      body.unit_price ?? 0,
      body.total_tons ?? 0,
      body.total_amount ?? 0,
      body.first_pay_pct ?? 80,
      body.first_pay_amount ?? 0,
      body.tail_pay_amount ?? 0,
      body.signed_at ?? null,
      body.status ?? 'active',
      body.note ?? null,
    );
    return c.json({ ok: true });
  });

  app.delete('/api/contracts', (c) => {
    const id = c.req.query('id');
    if (!id) return c.json({ ok: false, error: '缺少 id' }, 400);
    // Cascade delete inside a single transaction — payments + shipments + contract
    // must all go or none. Regression locked by test.
    const delPayments = db.prepare(`DELETE FROM payments WHERE contract_id = ?`);
    const delShipments = db.prepare(`DELETE FROM shipments WHERE contract_id = ?`);
    const delContract = db.prepare(`DELETE FROM contracts WHERE id = ?`);
    const cascade = db.transaction((cid: string) => {
      delPayments.run(cid);
      delShipments.run(cid);
      delContract.run(cid);
    });
    cascade(id);
    return c.json({ ok: true });
  });

  app.all('/api/contracts', (c) => c.json({ error: 'Method not allowed' }, 405));

  // ============================================================
  // /api/payments
  // ============================================================
  app.get('/api/payments', (c) => {
    const contractId = c.req.query('contract_id');
    let rows: PaymentRow[];
    if (contractId) {
      rows = db
        .prepare(
          `SELECT id, contract_id, kind, amount, paid_at, payer, method,
                  voucher_no, note, created_at
             FROM payments WHERE contract_id = ? ORDER BY paid_at DESC`,
        )
        .all(contractId) as PaymentRow[];
    } else {
      rows = db
        .prepare(
          `SELECT id, contract_id, kind, amount, paid_at, payer, method,
                  voucher_no, note, created_at
             FROM payments ORDER BY paid_at DESC`,
        )
        .all() as PaymentRow[];
    }
    return c.json({ payments: rows });
  });

  app.post('/api/payments', async (c) => {
    const parsed = await readJson<Partial<PaymentRow>>(c);
    if ('error' in parsed) return parsed.error;
    const body = parsed.body;
    // amount == null check — amount=0 must be accepted (regression).
    if (!body.id || !body.contract_id || body.amount == null || !body.paid_at) {
      return c.json({ ok: false, error: '缺少必填字段' }, 400);
    }
    db.prepare(
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
    ).run(
      body.id,
      body.contract_id,
      body.kind ?? 'first',
      body.amount,
      body.paid_at,
      body.payer ?? null,
      body.method ?? null,
      body.voucher_no ?? null,
      body.note ?? null,
    );
    return c.json({ ok: true });
  });

  app.delete('/api/payments', (c) => {
    const id = c.req.query('id');
    if (!id) return c.json({ ok: false, error: '缺少 id' }, 400);
    db.prepare(`DELETE FROM payments WHERE id = ?`).run(id);
    return c.json({ ok: true });
  });

  app.all('/api/payments', (c) => c.json({ error: 'Method not allowed' }, 405));

  // ============================================================
  // /api/shipments
  //
  // assay handling: worker accepts assay_json (already a JSON string)
  // straight through. Spec says "assay maps to assay_json (JSON encode)".
  // To stay backwards-compatible AND honour the spec, accept either:
  //   - body.assay_json (string, preferred — what the worker accepted), or
  //   - body.assay (object) → JSON.stringify and store as assay_json.
  // ============================================================

  interface ShipmentBody extends Partial<ShipmentRow> {
    assay?: unknown;
  }

  app.get('/api/shipments', (c) => {
    const contractId = c.req.query('contract_id');
    let rows: ShipmentRow[];
    if (contractId) {
      rows = db
        .prepare(
          `SELECT id, contract_id, vehicle_no, net_tons, gross_tons, tare_tons,
                  shipped_at, arrived_at, settled_at, settled_amount,
                  assay_json, status, note, created_at
             FROM shipments WHERE contract_id = ? ORDER BY shipped_at DESC`,
        )
        .all(contractId) as ShipmentRow[];
    } else {
      rows = db
        .prepare(
          `SELECT id, contract_id, vehicle_no, net_tons, gross_tons, tare_tons,
                  shipped_at, arrived_at, settled_at, settled_amount,
                  assay_json, status, note, created_at
             FROM shipments ORDER BY shipped_at DESC`,
        )
        .all() as ShipmentRow[];
    }
    return c.json({ shipments: rows });
  });

  app.post('/api/shipments', async (c) => {
    const parsed = await readJson<ShipmentBody>(c);
    if ('error' in parsed) return parsed.error;
    const body = parsed.body;
    // net_tons == null check — net_tons=0 must be accepted (regression).
    if (
      !body.id ||
      !body.contract_id ||
      body.net_tons == null ||
      !body.shipped_at
    ) {
      return c.json({ ok: false, error: '缺少必填字段' }, 400);
    }

    // Prefer explicit assay_json (string) for backwards-compat with worker.
    // Fall back to assay (object) → encode.
    let assayJson: string | null = null;
    if (body.assay_json !== undefined && body.assay_json !== null) {
      assayJson = body.assay_json;
    } else if (body.assay !== undefined && body.assay !== null) {
      assayJson = JSON.stringify(body.assay);
    }

    db.prepare(
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
    ).run(
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
      assayJson,
      body.status ?? 'shipped',
      body.note ?? null,
    );
    return c.json({ ok: true });
  });

  app.delete('/api/shipments', (c) => {
    const id = c.req.query('id');
    if (!id) return c.json({ ok: false, error: '缺少 id' }, 400);
    db.prepare(`DELETE FROM shipments WHERE id = ?`).run(id);
    return c.json({ ok: true });
  });

  app.all('/api/shipments', (c) => c.json({ error: 'Method not allowed' }, 405));

  // ============================================================
  // 404 for any other /api/* path (test expects 404 on unknown route)
  // ============================================================
  app.all('/api/*', (c) => c.json({ error: 'Not found' }, 404));

  return app;
}
