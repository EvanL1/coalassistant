import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initializeCloudStorage } from "../cloudStorage";
import { getCoalPrefs, getUserCoals } from "../storage";
import type { MasterCoalEntry } from "../types";
import { resolveCoalPool, toEstimateCoal } from "./resolvedCoal";
import poolCasesRaw from "../../../blend_kit_server/data/pool_cases.json?raw";

/**
 * 与服务端共享的煤池解析用例 (`blend_kit_server/data/pool_cases.json`)。
 *
 * 同一份数据 `today.rs::tests::shared_pool_cases_match_fixture` 也跑一遍: 手机 Agent 的
 * 求解请求由 today.rs 组装, 它是这里规则的镜像, 两端分叉时手机与网页会算出不同的配方。
 *
 * 用例里的偏好与自建煤是服务端存的那一份, 这里照网页今日屏的真实路径走: 云端状态经
 * `initializeCloudStorage` 写进本地缓存, 再由 `getCoalPrefs()` / `getUserCoals()` 读出
 * (两者会归一化坏数据), 最后才交给 `resolveCoalPool`。
 */
interface PoolCase {
  name: string;
  master_coals: unknown[];
  user_coals: unknown[];
  prefs: Record<string, unknown>;
  expected: [string, string][];
  props: Record<string, Record<string, number>>;
  overridden?: Record<string, string[]>;
}

const fixture = JSON.parse(poolCasesRaw) as { cases: PoolCase[] };

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

/** 按网页登录后的路径载入服务端存的状态, 返回今日屏求解前读到的偏好与自建煤。 */
async function loadLikeWeb(testCase: PoolCase) {
  vi.mocked(fetch).mockResolvedValueOnce(
    Response.json({
      initialized: true,
      coal_prefs: testCase.prefs,
      contract: null,
      quantity: 3700,
      user_coals: testCase.user_coals,
      shovel_tons: 3,
    }),
  );
  const cleanup = await initializeCloudStorage();
  cleanup();
  return { prefs: getCoalPrefs(), userCoals: getUserCoals() };
}

describe("与服务端共享的煤池解析用例", () => {
  it("用例数量足够覆盖每条规则", () => {
    expect(fixture.cases.length).toBeGreaterThanOrEqual(12);
  });

  it.each(fixture.cases.map((c) => [c.name, c] as const))("%s", async (_name, testCase) => {
    const { prefs, userCoals } = await loadLikeWeb(testCase);
    const pool = resolveCoalPool(
      testCase.master_coals as MasterCoalEntry[],
      userCoals,
      prefs,
    );
    expect(pool.map((coal) => [coal.name, coal.readiness])).toEqual(testCase.expected);
    for (const coal of pool) {
      expect(Object.keys(coal.overriddenProps).sort(), coal.name).toEqual(
        testCase.overridden?.[coal.name] ?? [],
      );
    }

    const solved = pool
      .map((coal) => toEstimateCoal(coal))
      .filter((coal) => coal != null);
    const ready = testCase.expected
      .filter(([, readiness]) => readiness === "ready")
      .map(([name]) => name);
    expect(solved.map((coal) => coal.name)).toEqual(ready);
    for (const coal of solved) {
      expect(coal.props).toEqual(testCase.props[coal.name]);
    }
  });
});
