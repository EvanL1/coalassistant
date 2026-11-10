import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getSession, LoginRateLimitError, logout, tryLogin } from "./auth";

/** 可清空的 localStorage 替身 (auth 认领/清空都走它). */
function stubLocalStorage(values: Map<string, string>) {
  vi.stubGlobal("localStorage", {
    get length() { return values.size; },
    key: (i: number) => [...values.keys()][i] ?? null,
    getItem: (k: string) => values.get(k) ?? null,
    setItem: (k: string, v: string) => void values.set(k, v),
    removeItem: (k: string) => void values.delete(k),
    clear: () => values.clear(),
  });
}

beforeEach(() => {
  vi.stubGlobal("window", new EventTarget());
  vi.stubGlobal("fetch", vi.fn());
  if (typeof CustomEvent === "undefined") {
    vi.stubGlobal(
      "CustomEvent",
      class extends Event {
        constructor(type: string) {
          super(type);
        }
      },
    );
  }
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("服务端认证", () => {
  it("从 HttpOnly 会话接口读取登录状态与角色", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response('{"authenticated":true,"admin":true}', {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    await expect(getSession()).resolves.toEqual({ authenticated: true, admin: true });
    expect(fetch).toHaveBeenCalledWith("/api/auth/session", {
      credentials: "same-origin",
      cache: "no-store",
    });
  });

  it("旧版服务端不返回 admin 时按普通用户处理", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response('{"authenticated":true}', { status: 200 }),
    );

    await expect(getSession()).resolves.toEqual({ authenticated: true, admin: false });
  });

  it("未登录时 admin 标记一律无效", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response('{"authenticated":false,"admin":true}', { status: 200 }),
    );

    await expect(getSession()).resolves.toEqual({ authenticated: false, admin: false });
  });

  it("会话接口出错视为未登录", async () => {
    vi.mocked(fetch).mockRejectedValue(new Error("network"));

    await expect(getSession()).resolves.toEqual({ authenticated: false, admin: false });
  });

  it("登录只把凭据发给同源服务", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response('{"authenticated":true}', { status: 200 }),
    );

    await expect(tryLogin(" doudou ", "secret")).resolves.toBe(true);
    expect(fetch).toHaveBeenCalledWith("/api/auth/login", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username: "doudou", password: "secret" }),
    });
  });

  it("登录成功后本机数据记到这个账号名下; 密码错误不认领", async () => {
    const values = new Map<string, string>([["doudou_blend.owner.v1", "张三"], ["doudou_blend.contract.v1", "[]"]]);
    stubLocalStorage(values);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    vi.mocked(fetch).mockResolvedValueOnce(new Response(null, { status: 401 }));
    await expect(tryLogin("李四", "错")).resolves.toBe(false);
    expect(values.get("doudou_blend.owner.v1")).toBe("张三");

    vi.mocked(fetch).mockResolvedValueOnce(new Response("{}", { status: 200 }));
    await expect(tryLogin(" 李四 ", "对")).resolves.toBe(true);
    expect(values.get("doudou_blend.owner.v1")).toBe("李四");
    expect(values.has("doudou_blend.contract.v1")).toBe(false);
  });

  it("管理员登录不认领、也不清本机的用户数据, 不弹换号确认", async () => {
    const values = new Map<string, string>([
      ["doudou_blend.owner.v1", "doudou"],
      ["doudou_blend.penalty_template.v1", "{\"clauses\":[]}"],
    ]);
    stubLocalStorage(values);
    const confirmSpy = vi.fn();
    vi.stubGlobal("confirm", confirmSpy);
    vi.mocked(fetch).mockResolvedValueOnce(new Response('{"admin":true}', { status: 200 }));

    await expect(tryLogin("admin", "secret")).resolves.toBe(true);

    expect(values.get("doudou_blend.owner.v1")).toBe("doudou");
    expect(values.get("doudou_blend.penalty_template.v1")).toBe('{"clauses":[]}');
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it("换普通账号登录要先确认会丢什么; 用户取消则退回未登录, 本机不动", async () => {
    const values = new Map<string, string>([
      ["doudou_blend.owner.v1", "张三"],
      ["doudou_blend.contract.v1", "[]"],
      ["doudou_blend.penalty_template.v1", "{\"clauses\":[]}"],
    ]);
    stubLocalStorage(values);
    const confirmSpy = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirmSpy);
    vi.mocked(fetch)
      .mockResolvedValueOnce(new Response("{}", { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));

    await expect(tryLogin("李四", "对")).rejects.toThrow("已取消登录");

    const message = confirmSpy.mock.calls[0]?.[0] as string;
    expect(message).toContain("张三");
    expect(message).toContain("扣款模板");
    expect(message).toContain("还没同步");
    // 服务端会话已成立, 必须退掉, 不然界面一刷新就进了李四的账号.
    expect(vi.mocked(fetch).mock.calls[1]?.[0]).toBe("/api/auth/logout");
    expect(values.get("doudou_blend.owner.v1")).toBe("张三");
    expect(values.has("doudou_blend.contract.v1")).toBe(true);
    expect(values.has("doudou_blend.penalty_template.v1")).toBe(true);
  });

  it("确认换号后才清掉旧账号数据", async () => {
    const values = new Map<string, string>([
      ["doudou_blend.owner.v1", "张三"],
      ["doudou_blend.contract.v1", "[]"],
    ]);
    stubLocalStorage(values);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    vi.mocked(fetch).mockResolvedValueOnce(new Response("{}", { status: 200 }));

    await expect(tryLogin("李四", "对")).resolves.toBe(true);

    expect(values.get("doudou_blend.owner.v1")).toBe("李四");
    expect(values.has("doudou_blend.contract.v1")).toBe(false);
  });

  it("登录被限速 (429): 带上 Retry-After 秒数, 没有头时用服务端 reason", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(
      new Response('{"reason":"稍后重试"}', { status: 429, headers: { "Retry-After": "30" } }),
    );
    const timed = await tryLogin("doudou", "x").catch((error: unknown) => error);
    expect(timed).toBeInstanceOf(LoginRateLimitError);
    expect((timed as LoginRateLimitError).retryAfterSeconds).toBe(30);

    vi.mocked(fetch).mockResolvedValueOnce(
      new Response('{"reason":"尝试次数过多, 一分钟后再试"}', { status: 429 }),
    );
    const untimed = await tryLogin("doudou", "x").catch((error: unknown) => error);
    expect(untimed).toBeInstanceOf(LoginRateLimitError);
    expect((untimed as LoginRateLimitError).retryAfterSeconds).toBeNull();
    expect((untimed as Error).message).toBe("尝试次数过多, 一分钟后再试");
  });

  it("限速以外的登录失败抛出服务端给的 reason", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(
      new Response('{"ok":false,"reason":"登录服务维护中"}', { status: 503 }),
    );
    await expect(tryLogin("doudou", "x")).rejects.toThrow("登录服务维护中");
  });

  it("管理员退出不清本机的用户数据", async () => {
    const values = new Map<string, string>([
      ["doudou_blend.owner.v1", "doudou"],
      ["doudou_blend.coal_prefs.v1", "{\"甲\":{\"enabled\":true}}"],
    ]);
    stubLocalStorage(values);
    vi.mocked(fetch)
      .mockResolvedValueOnce(Response.json({ authenticated: true, admin: true }))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));

    // 应用启动时查过会话, 知道这是管理员.
    await getSession();
    await logout();

    expect(values.has("doudou_blend.coal_prefs.v1")).toBe(true);
    expect(values.get("doudou_blend.owner.v1")).toBe("doudou");
  });

  it("管理员退出时断网, 也不清本机的用户数据", async () => {
    const values = new Map<string, string>([
      ["doudou_blend.owner.v1", "doudou"],
      ["doudou_blend.coal_prefs.v1", "{\"甲\":{\"enabled\":true}}"],
    ]);
    stubLocalStorage(values);
    vi.mocked(fetch)
      .mockResolvedValueOnce(Response.json({ authenticated: true, admin: true }))
      .mockRejectedValueOnce(new Error("断网"));

    await getSession();
    await logout().catch(() => undefined);

    expect(values.has("doudou_blend.coal_prefs.v1")).toBe(true);
  });

  it("退出登录清掉本机的应用数据", async () => {
    const values = new Map<string, string>([["doudou_blend.coal_prefs.v1", "{}"], ["别的", "留"]]);
    vi.stubGlobal("localStorage", {
      get length() { return values.size; },
      key: (i: number) => [...values.keys()][i] ?? null,
      getItem: (k: string) => values.get(k) ?? null,
      setItem: (k: string, v: string) => void values.set(k, v),
      removeItem: (k: string) => void values.delete(k),
      clear: () => values.clear(),
    });
    vi.mocked(fetch).mockRejectedValue(new Error("断网"));
    await expect(logout()).rejects.toThrow("断网");
    expect([...values.keys()]).toEqual(["别的"]);
  });

  it("退出请求结束后统一通知界面刷新", async () => {
    vi.mocked(fetch).mockResolvedValue(new Response(null, { status: 200 }));
    let changes = 0;
    window.addEventListener("doudou:auth_changed", () => {
      changes += 1;
    });

    await logout();

    expect(changes).toBe(1);
  });
});
