import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { forceBackend } from "./backend";

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("blend-kit-wasm", () => ({
  default: vi.fn(async () => undefined),
  solveJson: vi.fn(),
  getMasterJson: vi.fn(),
  getVersion: vi.fn(),
}));

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
  invoke.mockReset();
  vi.stubGlobal("window", new EventTarget());
  vi.stubGlobal("localStorage", makeStorage());
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
  it("Tauri 历史数量复用 SQLite db_status", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "db_status") return { history: 7 };
      return undefined;
    });

    const backend = await forceBackend("tauri");

    await expect(backend.countHistory()).resolves.toBe(7);
    expect(invoke).toHaveBeenCalledWith("db_status");
  });

  it("Tauri 仅在清空成功后派发统一历史变更事件", async () => {
    const backend = await forceBackend("tauri");
    let changes = 0;
    window.addEventListener("doudou:history_changed", () => {
      changes += 1;
    });

    invoke.mockResolvedValueOnce(undefined);
    await backend.clearHistory();
    expect(changes).toBe(1);

    invoke.mockRejectedValueOnce(new Error("SQLite 写入失败"));
    await expect(backend.clearHistory()).rejects.toThrow("SQLite 写入失败");
    expect(changes).toBe(1);
  });

  it("Web 历史数量读取 localStorage 中的实际记录", async () => {
    localStorage.setItem(
      "doudou_blend.history.v1",
      JSON.stringify([{ id: "1" }, { id: "2" }]),
    );

    const backend = await forceBackend("wasm");

    await expect(backend.countHistory()).resolves.toBe(2);
  });
});
