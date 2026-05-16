/**
 * /api/coals - 用户新增煤种 CRUD
 *
 * GET  /api/coals          → { coals: MasterCoalEntry[] }
 * POST /api/coals          → 新增, body: MasterCoalEntry
 * POST /api/coals/migrate  → 一次性导入, body: { coals: [...] } (用于从 localStorage 迁移)
 *
 * 注: middleware 已经做了鉴权, 这里直接处理业务.
 */

interface Env {
  DB: D1Database;
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

export const onRequestGet: PagesFunction<Env> = async ({ env }) => {
  const rows = await env.DB.prepare(
    `SELECT name, region, coal_type, status, props_json, fob, frt, note
       FROM user_coals
       ORDER BY updated_at DESC`,
  ).all<CoalRow>();

  const coals: MasterCoalEntry[] = (rows.results ?? []).map(rowToEntry);
  return json({ coals });
};

export const onRequestPost: PagesFunction<Env> = async ({ request, env }) => {
  const url = new URL(request.url);

  // /api/coals/migrate - 一次性批量导入
  if (url.pathname.endsWith("/migrate")) {
    let body: { coals?: MasterCoalEntry[] };
    try {
      body = await request.json();
    } catch {
      return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
    }
    const list = body.coals ?? [];
    let imported = 0;
    for (const c of list) {
      const ok = await upsertCoal(env.DB, c);
      if (ok) imported += 1;
    }
    return json({ ok: true, imported });
  }

  // /api/coals - 单条新增
  let coal: MasterCoalEntry;
  try {
    coal = await request.json();
  } catch {
    return json({ ok: false, error: "请求体不是合法 JSON" }, 400);
  }
  if (!coal.name || !coal.name.trim()) {
    return json({ ok: false, error: "煤名不能为空" }, 400);
  }
  const inserted = await insertCoal(env.DB, coal);
  if (!inserted) {
    return json({ ok: false, error: "煤种已存在或写入失败" }, 409);
  }
  return json({ ok: true });
};

export const onRequestDelete: PagesFunction<Env> = async ({ request, env }) => {
  const url = new URL(request.url);
  // 删除按名字, 走 query string ?name=xxx (避免动态路由文件)
  const name = url.searchParams.get("name");
  if (!name) {
    return json({ ok: false, error: "缺少 name" }, 400);
  }
  await env.DB.prepare(`DELETE FROM user_coals WHERE name = ?`).bind(name).run();
  return json({ ok: true });
};

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

async function insertCoal(db: D1Database, c: MasterCoalEntry): Promise<boolean> {
  try {
    await db
      .prepare(
        `INSERT INTO user_coals
           (name, region, coal_type, status, props_json, fob, frt, note)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
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
    return true;
  } catch {
    return false; // 主键冲突等
  }
}

/** upsert 用于 migrate (已存在则覆盖) */
async function upsertCoal(db: D1Database, c: MasterCoalEntry): Promise<boolean> {
  try {
    await db
      .prepare(
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
    return true;
  } catch {
    return false;
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Access-Control-Allow-Origin": "*",
    },
  });
}
