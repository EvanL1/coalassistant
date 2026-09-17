import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApiKey, listApiKeys, revokeApiKey } from "./apiKeys";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

beforeEach(() => {
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("数据更新密钥管理", () => {
  it("列表走同源 Cookie, 不缓存", async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ ok: true, keys: [] }));

    await expect(listApiKeys()).resolves.toEqual([]);
    expect(fetch).toHaveBeenCalledWith("/api/admin/keys", {
      credentials: "same-origin",
      cache: "no-store",
    });
  });

  it("创建返回明文, 且明文不写入任何本地存储", async () => {
    const plaintext = "dk_deadbeefdeadbeefdeadbeefdeadbeef";
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        ok: true,
        plaintext,
        key: {
          id: "k1",
          name: "张三",
          key_prefix: "dk_deadbee",
          created_at: "2026-09-17T00:00:00Z",
          last_used_at: null,
          revoked_at: null,
        },
      }),
    );

    const result = await createApiKey("  张三  ");
    expect(result.plaintext).toBe(plaintext);
    expect(fetch).toHaveBeenCalledWith("/api/admin/keys", {
      credentials: "same-origin",
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "  张三  " }),
    });
  });

  it("服务端返回的列表里不应含明文或哈希字段", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        ok: true,
        keys: [
          {
            id: "k1",
            name: "张三",
            key_prefix: "dk_deadbee",
            created_at: "2026-09-17T00:00:00Z",
            last_used_at: null,
            revoked_at: null,
          },
        ],
      }),
    );

    const [key] = await listApiKeys();
    expect(Object.keys(key)).toEqual([
      "id",
      "name",
      "key_prefix",
      "created_at",
      "last_used_at",
      "revoked_at",
    ]);
  });

  it("销毁对 id 做 URL 编码", async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ ok: true }));

    await revokeApiKey("a/b c");
    expect(fetch).toHaveBeenCalledWith("/api/admin/keys/a%2Fb%20c", {
      credentials: "same-origin",
      method: "DELETE",
    });
  });

  it("错误响应回显服务端的 reason, 而不是吞掉", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ ok: false, reason: "名字不能为空" }, 422),
    );

    await expect(createApiKey("")).rejects.toThrow("名字不能为空");
  });

  it("非 JSON 错误体原样抛出, 不会变成 undefined", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response("服务暂不可用", { status: 503 }),
    );

    await expect(listApiKeys()).rejects.toThrow("服务暂不可用");
  });
});
