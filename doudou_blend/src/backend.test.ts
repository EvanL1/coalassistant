import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { forceBackend } from "./backend";

function makeStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

beforeEach(() => {
  vi.stubGlobal("window", new EventTarget());
  vi.stubGlobal("localStorage", makeStorage());
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

describe("Backend 历史契约", () => {
  it("Web 历史数量读取 PostgreSQL API", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response('{"count":2}', {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const backend = await forceBackend();

    await expect(backend.countHistory()).resolves.toBe(2);
    expect(fetch).toHaveBeenCalledWith("/api/history/count", undefined);
  });

  it("Web 保存历史后派发统一变更事件", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response('{"id":"history-1"}', {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const backend = await forceBackend();
    let changes = 0;
    window.addEventListener("doudou:history_changed", () => {
      changes += 1;
    });
    const result = {
      ok: true,
      recipe: { 测试煤: 1 },
      cost: { fob_per_ton: 900, frt_per_ton: 100, cif_per_ton: 1000 },
      orders: [],
      indicator_check: [],
      warnings: [],
    };

    await backend.saveHistory(result, "默认合同", 3700);

    expect(changes).toBe(1);
    expect(fetch).toHaveBeenCalledWith("/api/history", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        result,
        contract_name: "默认合同",
        quantity: 3700,
      }),
    });
  });

  it("Web 求解通过同源 Rust API 并保留 JSON 字符串边界", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValue(
      new Response('{"ok":true}', {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const backend = await forceBackend();
    const input = '{"coals":[],"specs":[]}';

    await expect(backend.solveJson(input)).resolves.toBe('{"ok":true}');
    expect(fetchMock).toHaveBeenCalledWith("/api/solve", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: input,
    });
  });

  it("Web API 错误不会被误当作求解结果", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response("服务暂不可用", { status: 503 }),
    );
    const backend = await forceBackend();

    await expect(backend.getMasterJson()).rejects.toThrow("服务暂不可用");
  });
});
