import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initializeCloudStorage } from "./cloudStorage";
import { getCoalPrefs, setCoalPref } from "./storage";

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
      class<T> extends Event {
        detail: T;

        constructor(type: string, init?: CustomEventInit<T>) {
          super(type);
          this.detail = init?.detail as T;
        }
      },
    );
  }
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Web PostgreSQL 状态同步", () => {
  it("已有云端状态会覆盖本地缓存并持续写回", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(
        Response.json({
          initialized: true,
          coal_prefs: { 云端煤: { enabled: true } },
          contract: null,
          quantity: 4200,
          user_coals: [],
        }),
      )
      .mockResolvedValue(Response.json({ initialized: true }));

    const cleanup = await initializeCloudStorage();

    expect(getCoalPrefs()).toEqual({ 云端煤: { enabled: true } });
    expect(localStorage.getItem("doudou_blend.quantity.v1")).toBe("4200");

    setCoalPref("云端煤", { enabled: false });
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(fetchMock.mock.calls[1]?.[0]).toBe("/api/storage");
    cleanup();
  });

  it("空数据库首次启动会上传本地数据并导入历史", async () => {
    localStorage.setItem(
      "doudou_blend.coal_prefs.v1",
      JSON.stringify({ 本地煤: { enabled: true } }),
    );
    localStorage.setItem(
      "doudou_blend.history.v1",
      JSON.stringify([
        {
          id: "legacy-1",
          occurred_at: "2026-07-29T00:00:00.000Z",
          cost_cif: 1000,
          recipe: { 本地煤: 1 },
          contract_name: "默认合同",
        },
      ]),
    );
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(
        Response.json({
          initialized: false,
          coal_prefs: {},
          contract: null,
          quantity: 3700,
          user_coals: [],
        }),
      )
      .mockResolvedValueOnce(Response.json({ initialized: true }))
      .mockResolvedValueOnce(Response.json({ imported: 1 }));

    const cleanup = await initializeCloudStorage();

    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/api/storage",
      "/api/storage",
      "/api/history/import",
    ]);
    expect(localStorage.getItem("doudou_blend.history.v1")).toBeNull();
    cleanup();
  });
});
