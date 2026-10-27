import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearUserCoals,
  getCoalPrefs,
  getUserCoals,
  setCoalPref,
} from "./storage";

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

describe("storage 持久化数据防御", () => {
  it("损坏的容器结构不会让煤池读取崩溃", () => {
    localStorage.setItem("doudou_blend.coal_prefs.v1", "null");
    localStorage.setItem("doudou_blend.user_coals.v1", "{}");

    expect(getCoalPrefs()).toEqual({});
    expect(getUserCoals()).toEqual([]);
  });

  it("非法单煤偏好会被显式标记，而不是静默当作无覆盖", () => {
    localStorage.setItem(
      "doudou_blend.coal_prefs.v1",
      JSON.stringify({ 测试煤: "坏数据" }),
    );

    expect(getCoalPrefs().测试煤?.invalid_data).toBe(true);

    setCoalPref("测试煤", { enabled: true });
    expect(getCoalPrefs().测试煤).toMatchObject({ enabled: true });
    expect(getCoalPrefs().测试煤?.invalid_data).toBeUndefined();
  });

  it("批量清除用户煤时同步删除其偏好并保留 Master 偏好", () => {
    localStorage.setItem(
      "doudou_blend.user_coals.v1",
      JSON.stringify([
        { name: "用户煤", status: "draft", props: {} },
      ]),
    );
    localStorage.setItem(
      "doudou_blend.coal_prefs.v1",
      JSON.stringify({
        用户煤: { enabled: true, fob_override: 900 },
        Master煤: { enabled: false },
      }),
    );

    clearUserCoals(["Master煤"]);

    expect(getUserCoals()).toEqual([]);
    expect(getCoalPrefs()).toEqual({
      Master煤: { enabled: false },
    });
  });

  it("清理与 Master 重名的旧用户煤时保留共用偏好", () => {
    localStorage.setItem(
      "doudou_blend.user_coals.v1",
      JSON.stringify([
        { name: "同名煤", status: "draft", props: {} },
      ]),
    );
    localStorage.setItem(
      "doudou_blend.coal_prefs.v1",
      JSON.stringify({
        同名煤: { enabled: false, fob_override: 880 },
      }),
    );

    clearUserCoals([" 同名煤　"]);

    expect(getUserCoals()).toEqual([]);
    // 共用偏好保留; 旧版本录的价格键读取时摘掉 (煤价一律按质量估算).
    expect(getCoalPrefs()).toEqual({
      同名煤: { enabled: false },
    });
  });

  it("旧版本录的价格键读取时摘掉, 写回后本地缓存 (进而云端) 也就干净了", () => {
    localStorage.setItem(
      "doudou_blend.coal_prefs.v1",
      JSON.stringify({
        昌恒: { enabled: true, frt_override: 24, is_price_anchor: true },
      }),
    );
    expect(getCoalPrefs()).toEqual({ 昌恒: { enabled: true } });

    setCoalPref("昌恒", { hidden: false });
    const stored = JSON.parse(localStorage.getItem("doudou_blend.coal_prefs.v1")!);
    expect(stored.昌恒).not.toHaveProperty("frt_override");
    expect(stored.昌恒).not.toHaveProperty("is_price_anchor");
  });
});
