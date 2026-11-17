import { beforeEach, describe, expect, it, vi } from "vitest";
import { currentContract, estimateRequest, loadPriceFactorWithStatus, quoteRequest, type BlendBase } from "./blendRequest";
import { resolveCoal, toEstimateCoal } from "./resolvedCoal";
import type { MasterPriceFactor } from "../types";

const mocks = vi.hoisted(() => ({ benchmarks: vi.fn() }));
vi.mock("../benchmarks", async (original) => ({ ...(await original<typeof import("../benchmarks")>()), fetchBenchmarks: mocks.benchmarks }));
const resolvedA = resolveCoal({ name: "A", status: "verified", props: { S: 0.6 }, ...({ fob: 9999, frt: 999 } as object) }, null, "master");
const resolvedB = resolveCoal({ name: "B", status: "verified", props: { S: 0.8 } }, null, "master");
const base: BlendBase = { specs: [], totalQuantity: 900, shovelTons: 3, priceFactor: { adjusts: [] } };
beforeEach(() => { vi.resetAllMocks(); mocks.benchmarks.mockResolvedValue([]); });

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
    expect(request.specs).toEqual(base.specs);
    expect(request.truncate_decimal).toBe(true);
  });

  it("当前合同: 用户改过用用户的, 否则用 master 默认合同", () => {
    const master = { default_contract: { name: "默认合同", specs: [{ indicator: "S", direction: "Upper" as const, max: 0.8 }] } };
    expect(currentContract(null, master)).toEqual({ specs: master.default_contract.specs, name: "默认合同" });
    const mine = [{ indicator: "A", direction: "Upper" as const, max: 10 }];
    expect(currentContract(mine, master)).toEqual({ specs: mine, name: "用户自定义合同" });
    // 用户存了空合同也算用户的, 不退回默认.
    expect(currentContract([], master)).toEqual({ specs: [], name: "用户自定义合同" });
  });

  it("当前基准优先, 去掉 master 说明字段", async () => {
    mocks.benchmarks.mockResolvedValue([{ name: "当前", props: { S: 0.6 }, price: 2000, date: "2026-10-31", current_price: 2100 }]);
    const { priceFactor: factor } = await loadPriceFactorWithStatus({ adjusts: [], description: "说明" } as MasterPriceFactor);
    expect(factor.benchmarks?.[0].current_price).toBe(2100);
    expect(factor).not.toHaveProperty("description");
    expect(factor).not.toHaveProperty("market_price");
  });

  it("取不到当前基准用 master; 都没有就不另找市场价, 只带 master 自己配的", async () => {
    const benchmark = { name: "默认", props: { S: 0.6 }, price: 2000, date: "2026-10-30" };
    expect((await loadPriceFactorWithStatus({ adjusts: [], benchmarks: [benchmark] })).priceFactor.benchmarks).toEqual([benchmark]);
    const bare = (await loadPriceFactorWithStatus({ adjusts: [] })).priceFactor;
    expect(bare).not.toHaveProperty("benchmarks");
    expect(bare).not.toHaveProperty("market_price");
    expect((await loadPriceFactorWithStatus({ adjusts: [], market_price: 1800 })).priceFactor.market_price).toBe(1800);
  });

  it("基准报价取不到时退回内置基准并标出来, 取到了 (哪怕为空) 不标", async () => {
    const benchmark = { name: "默认", props: { S: 0.6 }, price: 2000, date: "2026-10-30" };
    mocks.benchmarks.mockRejectedValueOnce(new Error("服务异常"));
    const failed = await loadPriceFactorWithStatus({ adjusts: [], benchmarks: [benchmark] });
    expect(failed.benchmarksStale).toBe(true);
    expect(failed.priceFactor.benchmarks).toEqual([benchmark]);
    expect(mocks.benchmarks).toHaveBeenCalledWith({ strict: true });
    const ok = await loadPriceFactorWithStatus({ adjusts: [], benchmarks: [benchmark] });
    expect(ok.benchmarksStale).toBe(false);
  });
});
