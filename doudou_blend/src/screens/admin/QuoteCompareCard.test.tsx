// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { QuoteCompareCard } from "./QuoteCompareCard";
import type { QuoteRecord } from "../../types";

const mocks = vi.hoisted(() => ({ fetch: vi.fn(), remove: vi.fn(), confirm: vi.fn() }));
vi.mock("../../admin", () => ({ fetchQuoteComparison: mocks.fetch, deleteQuote: mocks.remove }));
const row: QuoteRecord = {
  id: "quote-1", username: "tester", batch_id: "batch-1", quoted_on: "2026-10-31", name: "测试煤", coal_type: "主焦煤",
  form: "精煤", region: "山西", fob: 2200, frt: 80, props: { S: 0.6 }, changed: ["S"],
  estimate: 2000, points: 1000, benchmark: "测试基准", created_at: "2026-10-31T00:00:00Z", gap: 200, ratio: 1.1,
};
const data = { quotes: [row], by_type: [{ coal_type: "主焦煤", count: 1, median_ratio: 1.1, median_gap: 200 }] };
beforeEach(() => { vi.resetAllMocks(); vi.stubGlobal("confirm", mocks.confirm); mocks.confirm.mockReturnValue(true); mocks.fetch.mockResolvedValue(data); mocks.remove.mockResolvedValue(undefined); });
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it("加载中到空态", async () => {
  let resolve!: (value: { quotes: []; by_type: [] }) => void;
  mocks.fetch.mockReturnValue(new Promise((done) => { resolve = done; }));
  render(<QuoteCompareCard />);
  expect(screen.getByText("加载中...")).toBeTruthy();
  await act(async () => resolve({ quotes: [], by_type: [] }));
  expect(screen.getByText("还没有询价")).toBeTruthy();
});

it("加载失败显示原因", async () => {
  mocks.fetch.mockRejectedValue(new Error("需要管理员权限"));
  render(<QuoteCompareCard />);
  expect(await screen.findByText("需要管理员权限")).toBeTruthy();
  expect(screen.queryByText("还没有询价")).toBeNull();
});

it("汇总与明细展示同口径数值及改过指标", async () => {
  render(<QuoteCompareCard />);
  const summary = await screen.findByRole("table", { name: "询价按煤种汇总" });
  expect(within(summary).getByText("主焦煤")).toBeTruthy();
  expect(within(summary).getByText("1.10")).toBeTruthy();
  expect(within(summary).getByText("+200")).toBeTruthy();
  const details = screen.getByRole("table", { name: "询价明细" });
  for (const text of ["2026-10-31", "测试煤", "主焦煤", "测试基准", "2200", "80", "2000", "+200", "1.10", "S"]) expect(within(details).getByText(text)).toBeTruthy();
});

it("无估价显示横线, 无煤种显示未标煤种", async () => {
  mocks.fetch.mockResolvedValue({ quotes: [{ ...row, estimate: null, benchmark: null, gap: null, ratio: null, coal_type: null, changed: [] }], by_type: [] });
  render(<QuoteCompareCard />);
  const table = await screen.findByRole("table", { name: "询价明细" });
  expect(within(table).getAllByText("—").length).toBeGreaterThanOrEqual(4);
  expect(within(table).getByText("未标煤种")).toBeTruthy();
});

it("删除先确认, 取消不调用, 确认后刷新明细与汇总", async () => {
  render(<QuoteCompareCard />);
  const button = await screen.findByRole("button", { name: "删除" });
  mocks.confirm.mockReturnValueOnce(false);
  fireEvent.click(button);
  expect(mocks.remove).not.toHaveBeenCalled();
  mocks.fetch.mockResolvedValue({ quotes: [], by_type: [] });
  fireEvent.click(button);
  expect(mocks.confirm).toHaveBeenCalledWith("删除这条询价? 删除后不进汇总");
  expect(await screen.findByText("还没有询价")).toBeTruthy();
  expect(mocks.remove).toHaveBeenCalledWith("quote-1");
  expect(mocks.fetch).toHaveBeenCalledTimes(2);
});

it("删除失败显示原因并保留明细", async () => {
  mocks.remove.mockRejectedValue(new Error("删除失败"));
  render(<QuoteCompareCard />);
  fireEvent.click(await screen.findByRole("button", { name: "删除" }));
  await waitFor(() => expect(mocks.remove).toHaveBeenCalled());
  expect(await screen.findByText("删除失败")).toBeTruthy();
  expect(screen.getByText("测试煤")).toBeTruthy();
});
