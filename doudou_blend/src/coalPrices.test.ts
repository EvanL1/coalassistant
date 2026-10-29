import { afterEach, describe, expect, it, vi } from "vitest";
import { cardPrice, coalsToEstimate, estimateLive, fetchCoalPrices } from "./coalPrices";
import { resolveCoal, resolveCoalPool } from "./domain/resolvedCoal";
import type { CoalPriceSnapshot, CoalValue, MasterCoalEntry } from "./types";

const base: MasterCoalEntry = {
  name: "测试煤", status: "verified", props: { S: 1.0, Y: 20 },
  coal_type: "主焦煤", form: "精煤", region: "山西", fob: 9999, frt: 888,
};
const coal = resolveCoal(base, null, "master");
const snapshot: CoalPriceSnapshot = {
  ok: true, day: "2026-10-28", prev_day: "2026-10-27", computed_at: "2026-10-28T09:00:03Z",
  prices: [{ name: base.name, price: 2202.4, prev_price: 2190.1, points: 1083.2,
    benchmark: "主焦基准", missing: ["M"] }],
};
const value: CoalValue = {
  name: base.name, fair: 2180, fob: null, adjustment: 0, points: 1000,
  gap: null, ratio: null, parts: [], missing: ["Y"], extrapolated: [],
};
const live = new Map([[base.name, value]]);

afterEach(() => vi.unstubAllGlobals());

describe("卡片估价", () => {
  it("共享煤按快照价格和前一日差值显示", () => {
    const price = cardPrice(coal, snapshot, live);
    expect(price.kind).toBe("snapshot");
    if (price.kind !== "snapshot") throw new Error("没有使用快照");
    expect(price.price).toBe(2202.4);
    expect(price.change).toBeCloseTo(12.3);
    expect(price.missing).toEqual(["M"]);
  });

  it("没有前一日价时不显示涨跌", () => {
    const first = { ...snapshot, prev_day: null,
      prices: [{ ...snapshot.prices[0], prev_price: undefined }] };
    expect(cardPrice(coal, first, live)).toEqual({
      kind: "snapshot", price: 2202.4, change: null, missing: ["M"],
    });
  });

  it.each([
    resolveCoal(base, { props_override: { S: 1.2 } }, "master"),
    resolveCoal(base, null, "user"),
  ])("改过化验值或用户新增的煤只用现算，不比较共享历史", (modified) => {
    expect(cardPrice(modified, snapshot, live)).toEqual({
      kind: "live", price: 2180, missing: ["Y"], modified: true,
    });
    expect(cardPrice(modified, snapshot, null)).toEqual({ kind: "unavailable" });
  });

  it("快照里没有该煤 (含整份快照取不到) 时现算，但不标按修改值估；没结果时不可用", () => {
    for (const empty of [{ ok: true, day: null, prices: [] }, null]) {
      expect(cardPrice(coal, empty, live)).toEqual({
        kind: "live", price: 2180, missing: ["Y"], modified: false,
      });
    }
    expect(cardPrice(coal, null, new Map())).toEqual({ kind: "unavailable" });
  });

  it("无效覆盖值不使用快照，也不送去现算", () => {
    const invalid = resolveCoal(base, { props_override: { S: -1 } }, "master");
    expect(cardPrice(invalid, snapshot, live)).toEqual({ kind: "unavailable" });
    expect(coalsToEstimate([invalid], null)).toEqual([]);
  });

  it.each([
    resolveCoal({ ...base, status: "archived" }, { props_override: { S: -1 } }, "master"),
    resolveCoal(base, { hidden: true, props_override: { S: -1 } }, "master"),
  ])("停用或隐藏不能遮蔽无效化验值", (invalid) => {
    expect(cardPrice(invalid, snapshot, live)).toEqual({ kind: "unavailable" });
    expect(cardPrice(invalid, null, live)).toEqual({ kind: "unavailable" });
    expect(coalsToEstimate([invalid], null)).toEqual([]);
  });

  it("名称冲突的煤不可用，但不影响其余修改煤的批量估价", () => {
    const pool = resolveCoalPool([base, { ...base, name: "正常煤" }], [base], {
      正常煤: { props_override: { S: 1.5 } },
    });
    const inputs = coalsToEstimate(pool, null);
    expect(inputs.map((input) => input.name)).toEqual(["正常煤"]);
    for (const duplicate of pool.filter((coal) => coal.name === base.name)) {
      expect(cardPrice(duplicate, snapshot, live)).toEqual({ kind: "unavailable" });
    }
    const normalLive = new Map([["正常煤", { ...value, name: "正常煤" }]]);
    expect(cardPrice(pool[2], null, normalLive)).toEqual({
      kind: "live", price: 2180, missing: ["Y"], modified: true,
    });
  });

  it("批量请求包含停用、缺项与修改煤，带有效化验值和分类，忽略报价", () => {
    const archived = resolveCoal({ ...base, name: "停用", status: "archived" }, null, "master");
    const incomplete = resolveCoal({ ...base, name: "缺项", status: "incomplete", props: { S: 1 } }, null, "master");
    const modified = resolveCoal(base, { props_override: { S: 1.2 } }, "master");
    const inputs = coalsToEstimate([coal, archived, incomplete, modified], snapshot);
    expect(inputs.map((input) => input.name)).toEqual(["停用", "缺项", base.name]);
    expect(inputs[2]).toEqual({ name: base.name, props: { S: 1.2, Y: 20 },
      coal_type: "主焦煤", form: "精煤", region: "山西", fob: 0, frt: 0 });
    expect(inputs[1].props).toEqual({ S: 1 });
  });

  it("只改启用或隐藏设置，仍用共享快照", () => {
    const disabled = resolveCoal(base, { enabled: false, hidden: true }, "master");
    expect(cardPrice(disabled, snapshot, live).kind).toBe("snapshot");
    expect(coalsToEstimate([disabled], snapshot)).toEqual([]);
  });
});

describe("估价接口", () => {
  it("快照请求带同源凭证且绕过缓存", async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(snapshot)));
    vi.stubGlobal("fetch", fetch);
    expect(await fetchCoalPrices()).toEqual(snapshot);
    expect(fetch).toHaveBeenCalledWith("/api/coal-prices", {
      credentials: "same-origin", cache: "no-store",
    });
  });

  it("一次 POST 打包所有现算煤，由服务端填系数与基准", async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ ok: true, coal_values: [value] })));
    vi.stubGlobal("fetch", fetch);
    const coals = [{ name: base.name, props: { S: 1 }, fob: 0, frt: 0 }];
    expect(await estimateLive(coals)).toEqual(live);
    expect(fetch).toHaveBeenCalledWith("/api/price", {
      method: "POST", credentials: "same-origin",
      headers: { "Content-Type": "application/json" }, body: JSON.stringify({ coals }),
    });
  });

  it.each([
    () => Promise.reject(new Error("离线")),
    () => Promise.resolve(new Response("", { status: 503 })),
    () => Promise.resolve(new Response("无效 JSON")),
    () => Promise.resolve(new Response(JSON.stringify({ ok: false, reason: "失败" }))),
    () => Promise.resolve(new Response(JSON.stringify({ ok: true }))),
  ])("接口失败或响应缺失时返回 null", async (response) => {
    vi.stubGlobal("fetch", vi.fn(response));
    expect(await fetchCoalPrices()).toBeNull();
    expect(await estimateLive([{ name: base.name, props: {}, fob: 0, frt: 0 }])).toBeNull();
  });

  it("没有需要现算的煤时不发送请求", async () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    expect(await estimateLive([])).toEqual(new Map());
    expect(fetch).not.toHaveBeenCalled();
  });
});
