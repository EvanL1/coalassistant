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
      return handleApi(req, env, url);
    }

    // 静态资源 (前端 SPA) - Cloudflare ASSETS 已经处理 index.html fallback
    return env.ASSETS.fetch(req);
  },
};

async function handleApi(req: Request, env: Env, url: URL): Promise<Response> {
  if (req.method === "OPTIONS") {
    return new Response(null, { status: 204, headers: corsHeaders() });
  }

  // /api/login 不需要鉴权
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
  } catch {
    return json({ ok: false, error: "煤种已存在或写入失败" }, 409);
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
