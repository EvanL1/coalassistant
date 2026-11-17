import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { listen } from "./events";
import {
  ApiError,
  apiFetch,
  apiJson,
  apiJsonOrNull,
  errorMessage,
  readOk,
  reasonFromBody,
  SESSION_EXPIRED_MESSAGE,
} from "./http";

beforeEach(() => {
  vi.stubGlobal("window", new EventTarget());
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("同源接口错误处理", () => {
  it("401 通知会话失效, 其他状态不通知", async () => {
    let expired = 0;
    listen(["session_expired"], () => {
      expired += 1;
    });
    vi.mocked(fetch)
      .mockResolvedValueOnce(new Response('{"ok":false,"reason":"请先登录"}', { status: 401 }))
      .mockResolvedValueOnce(new Response("{}", { status: 500 }));

    const first = await apiFetch("/api/solve", { method: "POST" });
    expect(first.status).toBe(401);
    expect(expired).toBe(1);
    await apiFetch("/api/master");
    expect(expired).toBe(1);
    // 同源 Cookie 会话在 apiFetch 里统一声明, 调用方不必各写一遍.
    expect(fetch).toHaveBeenNthCalledWith(1, "/api/solve", {
      credentials: "same-origin",
      method: "POST",
    });
    expect(vi.mocked(fetch).mock.calls[1]).toEqual(["/api/master", { credentials: "same-origin" }]);
  });

  it("错误响应只取 reason, 401 统一说登录已过期", async () => {
    await expect(
      readOk(new Response('{"ok":false,"reason":"请先登录"}', { status: 401 })),
    ).rejects.toThrow(SESSION_EXPIRED_MESSAGE);
    await expect(
      readOk(new Response('{"ok":false,"reason":"合同格式不对"}', { status: 400 })),
    ).rejects.toThrow(/^合同格式不对$/);
    await expect(readOk(new Response("好", { status: 200 }))).resolves.toBe("好");
  });

  it("apiJson: 2xx 解析 JSON, 空响应体给 null", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(new Response('{"ok":true,"n":3}', { status: 200 }));
    await expect(apiJson<{ n: number }>("/api/x")).resolves.toEqual({ ok: true, n: 3 });
    vi.mocked(fetch).mockResolvedValueOnce(new Response(null, { status: 204 }));
    await expect(apiJson("/api/x")).resolves.toBeNull();
  });

  it("apiJson: 非 2xx 抛带状态码的 ApiError, 消息是服务端给的原因", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(
      new Response('{"ok":false,"reason":"合同格式不对"}', { status: 400 }),
    );
    const error = await apiJson("/api/x").catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).status).toBe(400);
    expect((error as ApiError).message).toBe("合同格式不对");
  });

  it("apiJson: 2xx 但不是 JSON 时抛错, 不当成成功", async () => {
    vi.mocked(fetch).mockResolvedValueOnce(new Response("<html>网关页</html>", { status: 200 }));
    await expect(apiJson("/api/x")).rejects.toThrow();
  });

  it("apiJsonOrNull: 断网、非 2xx、不是 JSON 都给 null, 正常时给数据", async () => {
    vi.mocked(fetch).mockRejectedValueOnce(new TypeError("Failed to fetch"));
    await expect(apiJsonOrNull("/api/x")).resolves.toBeNull();
    vi.mocked(fetch).mockResolvedValueOnce(new Response('{"reason":"坏了"}', { status: 500 }));
    await expect(apiJsonOrNull("/api/x")).resolves.toBeNull();
    vi.mocked(fetch).mockResolvedValueOnce(new Response("不是 JSON", { status: 200 }));
    await expect(apiJsonOrNull("/api/x")).resolves.toBeNull();
    vi.mocked(fetch).mockResolvedValueOnce(new Response("[1,2]", { status: 200 }));
    await expect(apiJsonOrNull<number[]>("/api/x")).resolves.toEqual([1, 2]);
  });

  it("各种错误体都转成一句人话", () => {
    expect(reasonFromBody('{"errors":["甲 已有", "乙 已有"]}', 422)).toBe("甲 已有; 乙 已有");
    expect(reasonFromBody('{"ok":false}', 500)).toBe("服务返回错误 (HTTP 500)");
    expect(reasonFromBody("{坏的", 500)).toBe("服务返回错误 (HTTP 500)");
    expect(reasonFromBody("<html>502</html>", 502)).toBe("服务返回错误 (HTTP 502)");
    expect(reasonFromBody("", 503)).toBe("服务返回错误 (HTTP 503)");
    expect(reasonFromBody("服务暂不可用", 503)).toBe("服务暂不可用");
  });

  it("界面显示的错误不带 Error: 前缀, 不显示 JSON 原文", () => {
    expect(errorMessage(new Error("网络断了"))).toBe("网络断了");
    expect(errorMessage("Error: 网络断了")).toBe("网络断了");
    expect(errorMessage(new TypeError("Failed to fetch"))).toBe("Failed to fetch");
    expect(errorMessage(new Error('{"ok":false,"reason":"请先登录"}'))).toBe("请先登录");
    expect(errorMessage(new Error(""))).toBe("操作失败, 请重试");
  });
});
