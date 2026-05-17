/**
 * Server 集成测试 (Node.js + Hono + better-sqlite3 in-memory).
 *
 * 1:1 移植自 worker/index.test.ts. 黑盒走 app.request() 打完整入口,
 * 校验 status/body/Authorization. 每个测试用唯一 ID 避免相互污染.
 *
 * 跟 worker 测试模型一致: 整个文件共享一个 (db, app) 实例 (worker 那边
 * 也只有一个全局 env.DB), 用 uid() 隔离每个测试的数据.
 */
import Database from "better-sqlite3";
import { beforeAll, describe, expect, it } from "vitest";
import type { Hono } from "hono";

import { createApp } from "./app.js";
import { openInMemory } from "./db.js";

const PASS = "test-pass-123";

let db: Database.Database;
let app: Hono;

function uid(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

function authedInit(method: string, body?: unknown): RequestInit {
  const headers: Record<string, string> = {
    Authorization: `Bearer ${PASS}`,
  };
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
  }
  return {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  };
}

// 跟 worker 测试结构对齐: 先打一次 GET /api/customers 作为冒烟.
// 这里 openInMemory() 已经同步建好 schema, 不需要真正"触发", 但保留
// 这个 warmup 让结构跟 worker 测试 1:1 一致.
beforeAll(async () => {
  db = openInMemory();
  app = createApp({ db, authPass: PASS });
  const r = await app.request("/api/customers", {
    headers: { Authorization: `Bearer ${PASS}` },
  });
  expect(r.status).toBe(200);
});

// ============================================================
// 鉴权
// ============================================================
describe("auth", () => {
  it("无 token → 401", async () => {
    const r = await app.request("/api/customers");
    expect(r.status).toBe(401);
    expect(await r.json()).toMatchObject({ ok: false });
  });

  it("错 token → 401", async () => {
    const r = await app.request("/api/customers", {
      headers: { Authorization: "Bearer wrong-pass" },
    });
    expect(r.status).toBe(401);
  });

  it("/api/login 不需要 token", async () => {
    const r = await app.request("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ user: "doudou", pass: PASS }),
    });
    expect(r.status).toBe(200);
    const data = (await r.json()) as { ok: boolean; token: string };
    expect(data.ok).toBe(true);
    expect(data.token).toBe(PASS);
  });

  it("CORS preflight → 204", async () => {
    const r = await app.request("/api/customers", { method: "OPTIONS" });
    expect(r.status).toBe(204);
    expect(r.headers.get("Access-Control-Allow-Methods")).toContain("POST");
  });
});

// ============================================================
// /api/login
// ============================================================
describe("/api/login", () => {
  it("wrong user → 401", async () => {
    const r = await app.request("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ user: "wrong", pass: PASS }),
    });
    expect(r.status).toBe(401);
  });

  it("wrong pass → 401", async () => {
    const r = await app.request("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ user: "doudou", pass: "nope" }),
    });
    expect(r.status).toBe(401);
  });

  it("user 大小写不敏感, 密码 trim", async () => {
    const r = await app.request("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ user: "  DouDou ", pass: `  ${PASS}  ` }),
    });
    expect(r.status).toBe(200);
  });

  it("非 POST → 405", async () => {
    const r = await app.request("/api/login", { method: "GET" });
    expect(r.status).toBe(405);
  });

  it("非法 JSON → 400", async () => {
    const r = await app.request("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: "{not json",
    });
    expect(r.status).toBe(400);
  });
});

// ============================================================
// Schema bootstrap
// ============================================================
describe("schema bootstrap", () => {
  it("ensureSchema 把 7 张表全建出来", async () => {
    // openInMemory() 已经把 schema 建好. 直接查 sqlite_master.
    const rows = db
      .prepare(
        `SELECT name FROM sqlite_master WHERE type='table' ORDER BY name`,
      )
      .all() as { name: string }[];
    const tables = rows.map((r) => r.name);
    for (const t of [
      "contracts",
      "customers",
      "payments",
      "quotes",
      "shipments",
      "user_coals",
      "user_settings",
    ]) {
      expect(tables).toContain(t);
    }
  });

  it("ensureSchema 幂等 - 多次请求不报错", async () => {
    for (let i = 0; i < 3; i++) {
      const r = await app.request("/api/customers", {
        headers: { Authorization: `Bearer ${PASS}` },
      });
      expect(r.status).toBe(200);
    }
  });
});

// ============================================================
// /api/customers
// ============================================================
describe("/api/customers", () => {
  it("POST 缺 id → 400", async () => {
    const r = await app.request(
      "/api/customers",
      authedInit("POST", { name: "无 id" }),
    );
    expect(r.status).toBe(400);
  });

  it("POST 缺 name → 400", async () => {
    const r = await app.request(
      "/api/customers",
      authedInit("POST", { id: "x" }),
    );
    expect(r.status).toBe(400);
  });

  it("POST 空 name (全空格) → 400", async () => {
    const r = await app.request(
      "/api/customers",
      authedInit("POST", { id: "x", name: "   " }),
    );
    expect(r.status).toBe(400);
  });

  it("POST → GET 能拿到", async () => {
    const id = uid("c");
    const r = await app.request(
      "/api/customers",
      authedInit("POST", {
        id,
        name: "山东焦化",
        contact: "王经理",
        phone: "13800000000",
      }),
    );
    expect(r.status).toBe(200);

    const list = await app.request("/api/customers", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as { customers: Array<{ id: string; name: string; contact: string | null }> };
    const found = data.customers.find((c) => c.id === id);
    expect(found).toBeDefined();
    expect(found?.name).toBe("山东焦化");
    expect(found?.contact).toBe("王经理");
  });

  it("POST upsert - 重复 id 走 UPDATE 不报错", async () => {
    const id = uid("c");
    await app.request(
      "/api/customers",
      authedInit("POST", { id, name: "原名" }),
    );
    const r = await app.request(
      "/api/customers",
      authedInit("POST", { id, name: "改名", contact: "新人" }),
    );
    expect(r.status).toBe(200);

    const list = await app.request("/api/customers", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as { customers: Array<{ id: string; name: string; contact: string | null }> };
    const found = data.customers.find((c) => c.id === id);
    expect(found?.name).toBe("改名");
    expect(found?.contact).toBe("新人");
  });

  it("POST trim name", async () => {
    const id = uid("c");
    await app.request(
      "/api/customers",
      authedInit("POST", { id, name: "  带空格  " }),
    );
    const list = await app.request("/api/customers", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as { customers: Array<{ id: string; name: string }> };
    expect(data.customers.find((c) => c.id === id)?.name).toBe("带空格");
  });

  it("DELETE 缺 id → 400", async () => {
    const r = await app.request(
      "/api/customers",
      authedInit("DELETE"),
    );
    expect(r.status).toBe(400);
  });

  it("DELETE 真删", async () => {
    const id = uid("c");
    await app.request(
      "/api/customers",
      authedInit("POST", { id, name: "待删" }),
    );
    const r = await app.request(
      `/api/customers?id=${id}`,
      authedInit("DELETE"),
    );
    expect(r.status).toBe(200);

    const list = await app.request("/api/customers", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as { customers: Array<{ id: string }> };
    expect(data.customers.find((c) => c.id === id)).toBeUndefined();
  });
});

// ============================================================
// /api/quotes
// ============================================================
describe("/api/quotes", () => {
  it("POST 缺 customer_id → 400", async () => {
    const r = await app.request(
      "/api/quotes",
      authedInit("POST", { id: "q1", customer_name: "x" }),
    );
    expect(r.status).toBe(400);
  });

  it("POST → GET", async () => {
    const id = uid("q");
    const r = await app.request(
      "/api/quotes",
      authedInit("POST", {
        id,
        customer_id: "c1",
        customer_name: "客户A",
        recipe_json: '{"煤A":0.5}',
        cost_cif: 1000,
        quoted_price: 1100,
      }),
    );
    expect(r.status).toBe(200);
    const list = await app.request("/api/quotes", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as { quotes: Array<{ id: string; quoted_price: number }> };
    expect(data.quotes.find((q) => q.id === id)?.quoted_price).toBe(1100);
  });
});

// ============================================================
// /api/contracts + cascade
// ============================================================
describe("/api/contracts", () => {
  it("POST 缺 customer_id → 400", async () => {
    const r = await app.request(
      "/api/contracts",
      authedInit("POST", { id: "k1", customer_name: "x" }),
    );
    expect(r.status).toBe(400);
  });

  it("POST 默认值 - first_pay_pct=80, status=active", async () => {
    const id = uid("k");
    await app.request(
      "/api/contracts",
      authedInit("POST", {
        id,
        customer_id: "c1",
        customer_name: "客户",
        recipe_json: "{}",
        unit_price: 1000,
        total_tons: 100,
        total_amount: 100000,
        first_pay_amount: 80000,
        tail_pay_amount: 20000,
      }),
    );
    const list = await app.request("/api/contracts", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as { contracts: Array<{ id: string; first_pay_pct: number; status: string }> };
    const found = data.contracts.find((c) => c.id === id);
    expect(found?.first_pay_pct).toBe(80);
    expect(found?.status).toBe("active");
  });

  it("DELETE 级联清子表 (payments + shipments)", async () => {
    const contractId = uid("k");
    // 建合同
    await app.request(
      "/api/contracts",
      authedInit("POST", {
        id: contractId,
        customer_id: "c1",
        customer_name: "客户",
        recipe_json: "{}",
        unit_price: 1000,
        total_tons: 100,
        total_amount: 100000,
        first_pay_amount: 80000,
        tail_pay_amount: 20000,
      }),
    );
    // 加 1 笔 payment + 1 笔 shipment
    const payId = uid("p");
    await app.request(
      "/api/payments",
      authedInit("POST", {
        id: payId,
        contract_id: contractId,
        kind: "first",
        amount: 80000,
        paid_at: "2026-05-16",
      }),
    );
    const shipId = uid("s");
    await app.request(
      "/api/shipments",
      authedInit("POST", {
        id: shipId,
        contract_id: contractId,
        net_tons: 50,
        shipped_at: "2026-05-16",
      }),
    );

    // 删合同
    const r = await app.request(
      `/api/contracts?id=${contractId}`,
      authedInit("DELETE"),
    );
    expect(r.status).toBe(200);

    // 子表也应被清
    const pays = await app.request(
      `/api/payments?contract_id=${contractId}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    const payData = (await pays.json()) as { payments: unknown[] };
    expect(payData.payments).toHaveLength(0);

    const ships = await app.request(
      `/api/shipments?contract_id=${contractId}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    const shipData = (await ships.json()) as { shipments: unknown[] };
    expect(shipData.shipments).toHaveLength(0);
  });
});

// ============================================================
// /api/payments - bug regression: amount = 0 不该被拒
// ============================================================
describe("/api/payments", () => {
  it("amount=0 不应被当成缺字段 (regression)", async () => {
    const r = await app.request(
      "/api/payments",
      authedInit("POST", {
        id: uid("p"),
        contract_id: "k1",
        amount: 0,
        paid_at: "2026-05-16",
      }),
    );
    expect(r.status).toBe(200);
  });

  it("缺 paid_at → 400", async () => {
    const r = await app.request(
      "/api/payments",
      authedInit("POST", {
        id: uid("p"),
        contract_id: "k1",
        amount: 100,
      }),
    );
    expect(r.status).toBe(400);
  });

  it("按 contract_id 过滤", async () => {
    const c1 = uid("k");
    const c2 = uid("k");
    await app.request(
      "/api/payments",
      authedInit("POST", {
        id: uid("p"),
        contract_id: c1,
        amount: 100,
        paid_at: "2026-05-16",
      }),
    );
    await app.request(
      "/api/payments",
      authedInit("POST", {
        id: uid("p"),
        contract_id: c2,
        amount: 200,
        paid_at: "2026-05-16",
      }),
    );
    const list = await app.request(
      `/api/payments?contract_id=${c1}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    const data = (await list.json()) as { payments: Array<{ contract_id: string }> };
    expect(data.payments).toHaveLength(1);
    expect(data.payments[0].contract_id).toBe(c1);
  });
});

// ============================================================
// /api/shipments - net_tons = 0 是不合法的 (== null check)
// ============================================================
describe("/api/shipments", () => {
  it("net_tons 缺 → 400", async () => {
    const r = await app.request(
      "/api/shipments",
      authedInit("POST", {
        id: uid("s"),
        contract_id: "k1",
        shipped_at: "2026-05-16",
      }),
    );
    expect(r.status).toBe(400);
  });

  it("net_tons=0 是合法的 (==null 检查正确)", async () => {
    // 当前实现: !body.net_tons 会拒 0, 但 body.net_tons == null 不会.
    // 当前代码用的是 == null, 所以 0 应当被接受.
    const r = await app.request(
      "/api/shipments",
      authedInit("POST", {
        id: uid("s"),
        contract_id: "k1",
        net_tons: 0,
        shipped_at: "2026-05-16",
      }),
    );
    expect(r.status).toBe(200);
  });
});

// ============================================================
// /api/coals - 重名冲突 (bug regression: catch-all 误判 409)
// ============================================================
describe("/api/coals", () => {
  it("POST 重名 → 409 (UNIQUE 约束)", async () => {
    const name = `coal-${crypto.randomUUID()}`;
    const first = await app.request(
      "/api/coals",
      authedInit("POST", {
        name,
        status: "draft",
        props: { Ad: 10.5 },
      }),
    );
    expect(first.status).toBe(200);

    const dup = await app.request(
      "/api/coals",
      authedInit("POST", {
        name,
        status: "draft",
        props: {},
      }),
    );
    expect(dup.status).toBe(409);
    const body = (await dup.json()) as { error: string };
    expect(body.error).toContain("已存在");
  });

  it("POST 缺 name → 400", async () => {
    const r = await app.request(
      "/api/coals",
      authedInit("POST", { status: "draft", props: {} }),
    );
    expect(r.status).toBe(400);
  });

  it("POST 空 name (全空格) → 400", async () => {
    const r = await app.request(
      "/api/coals",
      authedInit("POST", { name: "   ", status: "draft", props: {} }),
    );
    expect(r.status).toBe(400);
  });

  it("POST → GET 拿得到, props 是对象", async () => {
    const name = `coal-${crypto.randomUUID()}`;
    await app.request(
      "/api/coals",
      authedInit("POST", {
        name,
        region: "山西",
        coal_type: "肥煤",
        status: "draft",
        props: { Ad: 8.5, Vd: 20.0 },
      }),
    );
    const list = await app.request("/api/coals", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as {
      coals: Array<{ name: string; region: string | null; props: Record<string, number> }>;
    };
    const found = data.coals.find((c) => c.name === name);
    expect(found?.region).toBe("山西");
    expect(found?.props).toEqual({ Ad: 8.5, Vd: 20.0 });
  });

  it("DELETE 真删", async () => {
    const name = `coal-${crypto.randomUUID()}`;
    await app.request(
      "/api/coals",
      authedInit("POST", { name, status: "draft", props: {} }),
    );
    const r = await app.request(
      `/api/coals?name=${encodeURIComponent(name)}`,
      authedInit("DELETE"),
    );
    expect(r.status).toBe(200);
    const list = await app.request("/api/coals", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    const data = (await list.json()) as { coals: Array<{ name: string }> };
    expect(data.coals.find((c) => c.name === name)).toBeUndefined();
  });
});

// ============================================================
// /api/coals/migrate
// ============================================================
describe("/api/coals/migrate", () => {
  it("批量导入 + 重复 UPSERT", async () => {
    const n1 = `coal-${crypto.randomUUID()}`;
    const n2 = `coal-${crypto.randomUUID()}`;
    const r = await app.request(
      "/api/coals/migrate",
      authedInit("POST", {
        coals: [
          { name: n1, status: "draft", props: { Ad: 1 } },
          { name: n2, status: "draft", props: { Ad: 2 } },
          { name: n1, status: "draft", props: { Ad: 9 } }, // dup → 走 UPDATE
        ],
      }),
    );
    expect(r.status).toBe(200);
    const data = (await r.json()) as { ok: boolean; imported: number };
    expect(data.imported).toBe(3);
  });
});

// ============================================================
// /api/settings (KV)
// ============================================================
describe("/api/settings", () => {
  it("GET 不存在 → value=null", async () => {
    const r = await app.request(
      `/api/settings?key=${uid("k")}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    expect(r.status).toBe(200);
    const data = (await r.json()) as { value: string | null };
    expect(data.value).toBeNull();
  });

  it("PUT 缺 key → 400", async () => {
    const r = await app.request(
      "/api/settings",
      authedInit("PUT", { value: "x" }),
    );
    expect(r.status).toBe(400);
  });

  it("PUT 空字符串 value → OK (typeof check 接受 '')", async () => {
    const key = uid("k");
    const r = await app.request(
      "/api/settings",
      authedInit("PUT", { key, value: "" }),
    );
    expect(r.status).toBe(200);

    const get = await app.request(
      `/api/settings?key=${key}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    const data = (await get.json()) as { value: string | null };
    expect(data.value).toBe("");
  });

  it("PUT → GET → DELETE → GET=null", async () => {
    const key = uid("k");
    await app.request(
      "/api/settings",
      authedInit("PUT", { key, value: '{"a":1}' }),
    );
    const get1 = await app.request(
      `/api/settings?key=${key}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    expect((await get1.json() as { value: string }).value).toBe('{"a":1}');

    await app.request(
      `/api/settings?key=${key}`,
      authedInit("DELETE"),
    );
    const get2 = await app.request(
      `/api/settings?key=${key}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    expect((await get2.json() as { value: string | null }).value).toBeNull();
  });

  it("PUT upsert 同 key", async () => {
    const key = uid("k");
    await app.request(
      "/api/settings",
      authedInit("PUT", { key, value: "v1" }),
    );
    await app.request(
      "/api/settings",
      authedInit("PUT", { key, value: "v2" }),
    );
    const get = await app.request(
      `/api/settings?key=${key}`,
      { headers: { Authorization: `Bearer ${PASS}` } },
    );
    expect((await get.json() as { value: string }).value).toBe("v2");
  });
});

// ============================================================
// 错误兜底 - fetch 层 try/catch
// ============================================================
describe("error wrapping", () => {
  it("非法 JSON → 400 (handler 自己处理)", async () => {
    const r = await app.request("/api/customers", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${PASS}`,
        "Content-Type": "application/json",
      },
      body: "{not json",
    });
    expect(r.status).toBe(400);
    const body = (await r.json()) as { error: string };
    expect(body.error).toContain("JSON");
  });

  it("未匹配路由 → 404", async () => {
    const r = await app.request("/api/nonexistent", {
      headers: { Authorization: `Bearer ${PASS}` },
    });
    expect(r.status).toBe(404);
  });

  it("未支持的 method → 405", async () => {
    const r = await app.request("/api/customers", {
      method: "PATCH",
      headers: { Authorization: `Bearer ${PASS}` },
    });
    expect(r.status).toBe(405);
  });

  it("D1 异常透出真实信息, 不再裸 500", async () => {
    // 故意制造一个底层异常: 删表 → INSERT 立刻爆 "no such table".
    // 顶层 onError 应当把它转成带 message 的 JSON 500 (而不是裸 500 没 body).
    db.prepare(`DROP TABLE IF EXISTS customers`).run();

    const r = await app.request(
      "/api/customers",
      authedInit("POST", { id: "x", name: "y" }),
    );
    expect(r.status).toBe(500);
    const body = (await r.json()) as { ok: boolean; error: string };
    expect(body.ok).toBe(false);
    expect(body.error).toMatch(/customers/i);

    // 把表建回去, 后面测试还得用
    db.prepare(
      `CREATE TABLE IF NOT EXISTS customers (
         id TEXT PRIMARY KEY,
         name TEXT NOT NULL,
         contact TEXT, phone TEXT, note TEXT,
         created_at TEXT NOT NULL DEFAULT (datetime('now')),
         updated_at TEXT NOT NULL DEFAULT (datetime('now')))`,
    ).run();
  });
});
