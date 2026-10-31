import { beforeEach, describe, expect, it, vi } from "vitest";
import { estimateRequest, loadPriceFactor, quoteRequest, type BlendBase } from "./blendRequest";
import { resolveCoal, toEstimateCoal } from "./resolvedCoal";
import type { MasterPriceFactor } from "../types";

const mocks = vi.hoisted(() => ({ benchmarks: vi.fn(), index: vi.fn() }));
vi.mock("../benchmarks", async (original) => ({ ...(await original<typeof import("../benchmarks")>()), fetchBenchmarks: mocks.benchmarks }));
vi.mock("../coal_index", () => ({ fetchLatestCoalIndex: mocks.index }));
const resolvedA = resolveCoal({ name: "A", status: "verified", props: { S: 0.6 }, fob: 9999, frt: 999 }, null, "master");
const resolvedB = resolveCoal({ name: "B", status: "verified", props: { S: 0.8 } }, null, "master");
const base: BlendBase = { specs: [], totalQuantity: 900, shovelTons: 3, priceFactor: { adjusts: [] } };
beforeEach(() => { vi.resetAllMocks(); mocks.benchmarks.mockResolvedValue([]); mocks.index.mockResolvedValue(1800); });

describe("请求拼装", () => {
  it("今日估价请求: 煤全在 extra_coals, 打开 estimate_unpriced", () => {
    const coals = [toEstimateCoal(resolvedA)!, toEstimateCoal(resolvedB)!];
    const request = estimateRequest(base, coals);
    expect(request.coals).toEqual([]);
    expect(request.price_factor?.extra_coals).toEqual(coals);
    expect(request.price_factor?.estimate_unpriced).toBe(true);
  });

  it("询价精算只用现场价格与指标, 不改煤池, 保留合同吨数及扣款条款", () => {
    const terms = { clauses: [] };
    const request = quoteRequest(base, [
      { coal: resolvedA, fob: 2150, frt: 80, props: {}, purchaseTerms: terms },
      { coal: resolvedB, fob: 1980, frt: 0, props: { S: 1.1 } },
      { coal: resolvedB, fob: 0, frt: 0, props: {} },
      { coal: resolvedB, fob: NaN, frt: 0, props: {} },
    ]);
    expect(request.coals.map((c) => [c.name, c.fob, c.frt])).toEqual([["A", 2150, 80], ["B", 1980, 0]]);
    expect(request.coals[0].purchase_terms).toEqual(terms);
    expect(request.coals[1].props.S).toBe(1.1);
    expect(resolvedB.props.S).toBe(0.8);
    expect(request.price_factor?.extra_coals).toEqual([]);
    expect(request.price_factor?.estimate_unpriced).toBe(false);
    expect(request.shovel_tons).toBe(3);
    expect(request.total_quantity).toBe(900);
    expect(request.specs).toBe(base.specs);
    expect(request.truncate_decimal).toBe(true);
  });

  it("当前基准优先, 去掉 master 说明字段", async () => {
    mocks.benchmarks.mockResolvedValue([{ name: "当前", props: { S: 0.6 }, price: 2000, date: "2026-10-31", current_price: 2100 }]);
    const factor = await loadPriceFactor({ adjusts: [], description: "说明" } as MasterPriceFactor);
    expect(factor.benchmarks?.[0].current_price).toBe(2100);
    expect(factor).not.toHaveProperty("description");
    expect(mocks.index).not.toHaveBeenCalled();
  });

  it("取不到当前基准用 master, 都没有才用指数", async () => {
    const benchmark = { name: "默认", props: { S: 0.6 }, price: 2000, date: "2026-10-30" };
    expect((await loadPriceFactor({ adjusts: [], benchmarks: [benchmark] })).benchmarks).toEqual([benchmark]);
    expect(mocks.index).not.toHaveBeenCalled();
    expect((await loadPriceFactor({ adjusts: [] })).market_price).toBe(1800);
  });
});
