/**
 * POST /api/login
 *
 * body: { user: string, pass: string }
 * resp: { ok: true, token: string } | { ok: false, error: string }
 *
 * 单租户共享密码:
 *   - user 字段保留 (前端体验跟旧的一致), 但服务器只校验 pass
 *   - 校验通过返回 token = AUTH_PASS, 客户端存起来作后续 API 调用的 Bearer
 */

interface Env {
  AUTH_PASS: string;
}

const AUTH_USER = "doudou";

export const onRequestPost: PagesFunction<Env> = async (context) => {
  const { request, env } = context;
  let body: { user?: string; pass?: string };
  try {
    body = await request.json();
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
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Access-Control-Allow-Origin": "*",
    },
  });
}
