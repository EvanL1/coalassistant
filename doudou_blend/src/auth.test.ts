import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getSession, logout, tryLogin } from "./auth";

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
