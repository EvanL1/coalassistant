// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { BlendResult, CoalMaster, CoalValue, Spec } from "../types";
import type { CoalPrefs } from "../storage";
import * as storage from "../storage";
import { QuoteScreen } from "./QuoteScreen";
import { TodayScreen } from "./TodayScreen";

const mocks = vi.hoisted(() => ({
  master: vi.fn(), solve: vi.fn(), save: vi.fn(), archive: vi.fn(), latest: vi.fn(), benchmarks: vi.fn(),
  prefs: {} as CoalPrefs, contract: null as Spec[] | null, quantity: 900,
}));
vi.mock("../master_loader", () => ({ loadMaster: mocks.master }));
vi.mock("../backend", () => ({ getBackend: async () => ({ solveJson: mocks.solve, saveHistory: mocks.save }) }));
vi.mock("../quotes", () => ({ archiveQuotes: mocks.archive, fetchLatestQuotes: mocks.latest }));
vi.mock("../benchmarks", async (original) => ({ ...(await original<typeof import("../benchmarks")>()), fetchBenchmarks: mocks.benchmarks }));
vi.mock("../coal_index", () => ({ fetchLatestCoalIndex: async () => null }));
vi.mock("../storage", async (original) => {
  const real = await original<typeof import("../storage")>();
  return {
    ...Object.fromEntries(Object.entries(real).map(([key, value]) => [key,
      typeof value === "function" && /^(set|clear|append|add|remove|enable|replace)/.test(key) ? vi.fn(value) : value])),
    getCoalPrefs: () => mocks.prefs, getUserContract: () => mocks.contract,
    getUserCoals: () => [], getQuantity: () => mocks.quantity, getShovelTons: () => 3,
  };
});

const master: CoalMaster = {
  version: "test", updated_at: "2026-10-31", description: "测试煤池",
  default_contract: { name: "测试合同", specs: [] },
  coals: ["甲", "乙", "丙", "丁"].map<CoalMaster["coals"][number]>((name) => ({
    name, status: "verified", coal_type: "主焦煤", form: "精煤", region: "山西",
    props: { S: 0.6, A: 9.8, V: 25, G: 85, Y: 20, petro: 50, CSR: 60, M: 10 },
    fob: 9999, frt: 888,
  })).concat([
    { name: "停用", status: "archived", props: { S: 1 } },
    { name: "坏指标", status: "verified", props: { S: -1 } },
  ] as CoalMaster["coals"]),
  price_factor: { adjusts: [], benchmarks: [{ name: "基准", price: 2000, props: { S: 0.6 }, date: "2026-10-30" }] },
};
const value = (name: string, blend_value: number): CoalValue => ({
  name, fair: 2000, points: 1000, adjustment: 0, fob: null, gap: null, ratio: null,
  parts: [], missing: [], extrapolated: [], blend_value, benchmark: "基准",
});
function result(price = 2100): BlendResult {
  return {
    ok: true, recipe: { 甲: 0.4, 乙: 0.6 },
    cost: { fob_per_ton: price, frt_per_ton: 0, cif_per_ton: price, total_fob: price * 900, total_frt: 0, total_cif: price * 900 },
    orders: [{ coal: "甲", ratio: 0.4, tons: 360 }, { coal: "乙", ratio: 0.6, tons: 540 }],
    indicator_check: [], warnings: [],
    coal_values: [value("甲", 2000), value("乙", 2000), value("丙", 1850), value("丁", 0)],
  };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function input(name: string) { return screen.getByLabelText(name) as HTMLInputElement; }
function fill(name: string, value: string) { fireEvent.change(input(name), { target: { value } }); }
function priceBoth() { fill("甲 出厂价", "2150"); fill("乙 出厂价", "1980"); }
async function open(todayResult = result()) {
  const back = vi.fn();
  const view = render(<QuoteScreen todayResult={todayResult} onBack={back} />);
  await screen.findByLabelText("添加煤");
  return { ...view, back };
}
function add(name: string) { fireEvent.change(screen.getByLabelText("添加煤"), { target: { value: name } }); }

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocks.prefs = {};
  mocks.contract = null;
  mocks.quantity = 900;
  mocks.master.mockResolvedValue(structuredClone(master));
  mocks.benchmarks.mockResolvedValue([]);
  mocks.latest.mockResolvedValue(new Map([["甲", { name: "甲", fob: 2150, frt: 80, quoted_on: "2026-10-28" }]]));
  mocks.archive.mockResolvedValue([]);
  mocks.solve.mockResolvedValue(JSON.stringify(result()));
  mocks.save.mockResolvedValue(undefined);
  vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000001");
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("默认煤按今日配方用量排序, 只可加启用有效煤, 能移除", async () => {
  await open();
  expect(screen.getAllByLabelText(/ 出厂价$/).map((el) => el.getAttribute("aria-label"))).toEqual(["乙 出厂价", "甲 出厂价"]);
  expect(screen.queryByRole("option", { name: "停用" })).toBeNull();
  expect(screen.queryByRole("option", { name: "坏指标" })).toBeNull();
  add("丙");
  expect(input("丙 出厂价")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "去掉 丙" }));
  expect(screen.queryByLabelText("丙 出厂价")).toBeNull();
});

it("输入始终为空, 最近询价只作 placeholder, 返回再进也为空", async () => {
  const view = await open();
  await waitFor(() => expect(input("甲 出厂价").placeholder).toBe("上次 2150 (10-28)"));
  expect(input("甲 出厂价").value).toBe("");
  expect(input("甲 运费").value).toBe("");
  expect(input("甲 运费").placeholder).toBe("上次 80 (10-28)");
  priceBoth();
  view.unmount();
  await open();
  expect(input("甲 出厂价").value).toBe("");
});

it("少于两种有效出厂价不能发请求", async () => {
  await open();
  expect((screen.getByRole("button", { name: "精算" }) as HTMLButtonElement).disabled).toBe(true);
  fill("甲 出厂价", "2150");
  fill("乙 出厂价", "0");
  fireEvent.click(screen.getByRole("button", { name: "精算" }));
  expect(mocks.solve).not.toHaveBeenCalled();
  expect(mocks.archive).not.toHaveBeenCalled();
  expect(screen.getByText("至少填两种煤的出厂价")).toBeTruthy();
});

it("阈值只显示给未入选且 blend_value 为有限正数的煤", async () => {
  const today = result();
  today.coal_values!.push(value("不存在", Infinity));
  await open(today);
  expect(screen.queryByText(/到厂价低于/)).toBeNull();
  add("丙"); add("丁");
  expect(screen.getAllByText(/到厂价低于/)).toHaveLength(1);
  expect(screen.getByText("到厂价低于 1850 元才会配进去")).toBeTruthy();
});

it("报价偏离超过 30% 提示高低, 不拦精算", async () => {
  await open();
  fill("甲 出厂价", "2600");
  expect(screen.queryByText(/请核对/)).toBeNull();
  fill("甲 出厂价", "2800"); fill("乙 出厂价", "1200");
  expect(screen.getByText("比今日估价高 40%, 请核对")).toBeTruthy();
  expect(screen.getByText("比今日估价低 40%, 请核对")).toBeTruthy();
  expect((screen.getByRole("button", { name: "精算" }) as HTMLButtonElement).disabled).toBe(false);
});

it("精算与存档只带本次填价煤和实际指标, 不读旧价格、不改煤池", async () => {
  mocks.prefs = { 甲: { fob_override: 1, frt_override: 2 } } as unknown as CoalPrefs;
  await open();
  add("丙");
  priceBoth(); fill("甲 运费", "80");
  fireEvent.click(screen.getByRole("button", { name: "改这批指标 甲" }));
  expect(screen.getAllByLabelText(/^甲 (S|A|V|G|Y|petro|CSR|M)$/)).toHaveLength(8);
  fill("甲 S", "1.1");
  expect(screen.getByText("S 已改")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await waitFor(() => expect(mocks.solve).toHaveBeenCalledTimes(1));
  const request = JSON.parse(mocks.solve.mock.calls[0][0]);
  expect(request.coals.map((c: { name: string; fob: number; frt: number }) => [c.name, c.fob, c.frt])).toEqual([["乙", 1980, 0], ["甲", 2150, 80]]);
  expect(request.coals[1].props.S).toBe(1.1);
  expect(request.price_factor.extra_coals).toEqual([]);
  expect(request.price_factor.estimate_unpriced).toBe(false);
  expect(request.total_quantity).toBe(900);
  expect(request.shovel_tons).toBe(3);
  const archived = mocks.archive.mock.calls[0][1];
  expect(archived.map((c: { name: string }) => c.name)).toEqual(["乙", "甲"]);
  expect(archived[1]).toMatchObject({ fob: 2150, frt: 80, props: { S: 1.1, A: 9.8 }, changed: ["S"] });
  expect(master.coals[0].props.S).toBe(0.6);
});

it("空或非数字指标沿用煤池值, 非数字提示且不存为改动", async () => {
  await open(); priceBoth();
  fireEvent.click(screen.getByRole("button", { name: "改这批指标 甲" }));
  fill("甲 S", ""); fill("甲 A", "abc");
  expect(screen.getByText(/A 不是有效数字/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await waitFor(() => expect(mocks.archive).toHaveBeenCalled());
  expect(mocks.archive.mock.calls[0][1][1]).toMatchObject({ props: { S: 0.6, A: 9.8 }, changed: [] });
});

it("运费非法时明确提示并禁用精算", async () => {
  await open(); priceBoth(); fill("甲 运费", "-1");
  expect(screen.getByText(/运费须/)).toBeTruthy();
  expect((screen.getByRole("button", { name: "精算" }) as HTMLButtonElement).disabled).toBe(true);
});

it("存档未完成或失败都不阻塞结果, 对比只列本批煤", async () => {
  const archive = deferred<[]>(); mocks.archive.mockReturnValue(archive.promise);
  const solved = result();
  solved.coal_values = [
    { ...value("甲", 2000), fob: 2150, gap: 150 },
    { ...value("乙", 2000), fob: 1980, gap: -20 }, value("丙", 1850),
  ];
  mocks.solve.mockResolvedValue(JSON.stringify(solved));
  await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  const table = await screen.findByRole("table", { name: "报价对比" });
  expect(within(table).getByText("高 150 元")).toBeTruthy();
  expect(within(table).getByText("低 20 元")).toBeTruthy();
  expect(within(table).queryByText("丙")).toBeNull();
  expect(screen.getByText("混合指标")).toBeTruthy();
  expect(screen.getByText("精算配方")).toBeTruthy();
  await act(async () => archive.reject(new Error("数据库尚未配置")));
  expect(await screen.findByText("询价未存档: 数据库尚未配置")).toBeTruthy();
  expect(screen.getByRole("table", { name: "报价对比" })).toBeTruthy();
});

it("无基准时不把本批报价反推的 fair 和 gap 当作估价对比", async () => {
  const solved = result();
  solved.coal_values = [
    { ...value("甲", 2500), benchmark: undefined, fob: 2000, fair: 2500, gap: -500 },
    { ...value("乙", 2500), benchmark: undefined, fob: 3000, fair: 2500, gap: 500 },
  ];
  mocks.solve.mockResolvedValue(JSON.stringify(solved));
  await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  const table = await screen.findByRole("table", { name: "报价对比" });
  expect(within(table).getByText("2000")).toBeTruthy();
  expect(within(table).getByText("3000")).toBeTruthy();
  expect(within(table).queryByText("2500")).toBeNull();
  expect(within(table).queryByText(/500 元/)).toBeNull();
  expect(within(table).getAllByText("—")).toHaveLength(4);
});

it("不可行时保留具体剔煤提示, 方便补齐指标后重算", async () => {
  mocks.solve.mockResolvedValue(JSON.stringify({ ...result(), ok: false, cost: null, orders: [], recipe: {},
    reason: "无可用煤", warnings: ["剔除 甲: 缺指标 胶质", "剔除 乙: 缺指标 胶质"],
  }));
  await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  expect(await screen.findByText("无可用煤")).toBeTruthy();
  expect(screen.getByText("剔除 甲: 缺指标 胶质")).toBeTruthy();
  expect(screen.getByText("剔除 乙: 缺指标 胶质")).toBeTruthy();
  expect(mocks.archive).toHaveBeenCalledTimes(1);
});

it("求解等待和不可行都照常存档", async () => {
  const solve = deferred<string>(); mocks.solve.mockReturnValue(solve.promise);
  await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await waitFor(() => expect(mocks.archive).toHaveBeenCalledTimes(1));
  await act(async () => solve.resolve(JSON.stringify({ ...result(), ok: false, cost: null, reason: "测试约束冲突", orders: [], recipe: {} })));
  expect(await screen.findByText("测试约束冲突")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "保存到历史" })).toBeNull();
});

it("乱序求解只显示最新, 同会话重复存档仍用同一个 batchId", async () => {
  const first = deferred<string>(); const second = deferred<string>();
  mocks.solve.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await waitFor(() => expect(mocks.solve).toHaveBeenCalledTimes(1));
  fill("甲 出厂价", "2180");
  fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await waitFor(() => expect(mocks.solve).toHaveBeenCalledTimes(2));
  await act(async () => second.resolve(JSON.stringify(result(2200))));
  await act(async () => first.resolve(JSON.stringify(result(1000))));
  expect(screen.getByText("2200", { selector: ".cost-int" })).toBeTruthy();
  expect(screen.queryByText("1000", { selector: ".cost-int" })).toBeNull();
  expect(mocks.archive).toHaveBeenCalledTimes(2);
  expect(mocks.archive.mock.calls.map((call) => call[0])).toEqual(Array(2).fill("00000000-0000-4000-8000-000000000001"));
});

it("连续精算按点击顺序存档, 旧存档失败不覆盖新反馈", async () => {
  const first = deferred<[]>(); mocks.archive.mockReturnValueOnce(first.promise);
  await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await screen.findByRole("table", { name: "报价对比" });
  fill("甲 出厂价", "2180"); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await screen.findByRole("table", { name: "报价对比" });
  expect(mocks.archive).toHaveBeenCalledTimes(1);
  await act(async () => first.reject(new Error("旧请求失败")));
  await waitFor(() => expect(mocks.archive).toHaveBeenCalledTimes(2));
  expect(mocks.archive.mock.calls[1][1][1].fob).toBe(2180);
  expect(screen.queryByText(/旧请求失败/)).toBeNull();
});

it("保存用求解快照的合同和吨数, 编辑后不可保存旧结果", async () => {
  await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  fireEvent.click(await screen.findByRole("button", { name: "保存到历史" }));
  expect(await screen.findByText("✓ 已保存")).toBeTruthy();
  expect(mocks.save).toHaveBeenCalledWith(result(), "测试合同", 900);
  fill("甲 出厂价", "2190");
  expect(screen.queryByRole("button", { name: "保存到历史" })).toBeNull();
});

it("保存失败展示反馈, 返回调用 onBack", async () => {
  mocks.save.mockRejectedValue(new Error("断网"));
  const { back } = await open(); priceBoth(); fireEvent.click(screen.getByRole("button", { name: "精算" }));
  fireEvent.click(await screen.findByRole("button", { name: "保存到历史" }));
  expect(await screen.findByText("保存失败")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "← 返回今日" }));
  expect(back).toHaveBeenCalledTimes(1);
});

it("精算与改指标不写 localStorage 或 storage, 不触发云同步事件", async () => {
  localStorage.setItem("保留", "原值");
  const before = { ...localStorage };
  const sync = vi.fn(); window.addEventListener("doudou:prefs_changed", sync);
  await open(); priceBoth();
  fireEvent.click(screen.getByRole("button", { name: "改这批指标 甲" })); fill("甲 S", "1.2");
  fireEvent.click(screen.getByRole("button", { name: "精算" }));
  await screen.findByRole("table", { name: "报价对比" });
  expect({ ...localStorage }).toEqual(before);
  for (const [name, fn] of Object.entries(storage)) {
    if (/^(set|clear|append|add|remove|enable|replace)/.test(name) && vi.isMockFunction(fn)) expect(fn).not.toHaveBeenCalled();
  }
  expect(sync).not.toHaveBeenCalled();
  window.removeEventListener("doudou:prefs_changed", sync);
});

it("今日入口打开精算页, 返回仍保留原估价结果", async () => {
  render(<TodayScreen onNavigate={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "询价精算" }));
  await screen.findByLabelText("添加煤");
  expect(screen.queryByText("今日配煤")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "← 返回今日" }));
  expect(screen.getByText("今日配煤")).toBeTruthy();
  expect(screen.getByText("2100", { selector: ".cost-int" })).toBeTruthy();
  expect(mocks.solve).toHaveBeenCalledTimes(1);
});

it("今日不可行也有询价入口, 重算时禁用", async () => {
  mocks.solve.mockResolvedValueOnce(JSON.stringify({ ...result(), ok: false, cost: null, reason: "测试不可行" }));
  render(<TodayScreen onNavigate={vi.fn()} />);
  const entry = await screen.findByRole("button", { name: "询价精算" }) as HTMLButtonElement;
  expect(entry.disabled).toBe(false);
  mocks.solve.mockReturnValue(new Promise(() => {}));
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(entry.disabled).toBe(true);
});

it("精算页开着时今日页后台重算失败, 已填的报价不丢", async () => {
  render(<TodayScreen onNavigate={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "询价精算" }));
  await screen.findByLabelText("添加煤");
  fill("甲 出厂价", "2150");
  mocks.solve.mockRejectedValueOnce(new Error("网络断了"));
  await act(async () => {
    window.dispatchEvent(new Event("doudou:prefs_changed"));
  });
  await waitFor(() => expect(mocks.solve).toHaveBeenCalledTimes(2));
  await act(async () => {});
  expect(screen.queryByText(/网络断了/)).toBeNull();
  expect(input("甲 出厂价").value).toBe("2150");
});

it("差额四舍五入到 0 元时显示持平, 不写高 0 元", async () => {
  const solved = result();
  solved.coal_values = [
    { ...value("甲", 2000), fob: 2000.3, gap: 0.3 },
    { ...value("乙", 2000), fob: 1999.6, gap: -0.4 },
  ];
  mocks.solve.mockResolvedValue(JSON.stringify(solved));
  await open();
  priceBoth();
  fireEvent.click(screen.getByRole("button", { name: "精算" }));
  const table = await screen.findByRole("table", { name: "报价对比" });
  expect(within(table).getAllByText("持平")).toHaveLength(2);
  expect(within(table).queryByText(/[高低] 0 元/)).toBeNull();
});
