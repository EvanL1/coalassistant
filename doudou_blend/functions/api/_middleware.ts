/**
 * Auth 中间件 - 应用到 /api/* 所有路由 (除了 /api/login).
 *
 * 单租户共享密码模型:
 *   - 服务器 secret: AUTH_PASS (wrangler pages secret put AUTH_PASS)
 *   - 客户端登录后存 token = AUTH_PASS 到 localStorage, 后续请求带
 *     Authorization: Bearer <token>
 *   - middleware 校验 Authorization 头是否等于 AUTH_PASS
 *
 * 已知限制: token 就是密码, 泄露 token 等于泄露密码.
 * 升级路径: 引入随机 session token + KV 存活检, 但本次先不做.
 */

interface Env {
  AUTH_PASS: string;
  DB: D1Database;
}

export const onRequest: PagesFunction<Env> = async (context) => {
  const { request, env, next } = context;
  const url = new URL(request.url);

  // /api/login 不需要鉴权
  if (url.pathname === "/api/login") {
    return next();
  }

  // CORS preflight
  if (request.method === "OPTIONS") {
    return new Response(null, { status: 204, headers: corsHeaders() });
  }

  const auth = request.headers.get("Authorization") || "";
  const m = auth.match(/^Bearer\s+(.+)$/);
  if (!m || m[1] !== env.AUTH_PASS) {
    return json({ ok: false, error: "未授权" }, 401);
  }

  return next();
};

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
