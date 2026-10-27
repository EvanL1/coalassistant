// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { McciCard } from "./McciCard";

const mocks = vi.hoisted(() => ({ fetchMcci: vi.fn() }));

vi.mock("../mcci", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../mcci")>()),
  fetchMcci: mocks.fetchMcci,
}));

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const fat = {
  code: "LJM_FM",
  type_name: "肥煤",
  name: "肥煤指数",
  coal_type: "肥煤",
  form: "精煤",
  region: "山西",
  props: { A: 10, V: 30, S: 1.8, G: 90, M: 10 },
  points: [
    { date: "2026-10-19", price: 2186.2 },
    { date: "2026-10-23", price: 2188.7 },
    { date: "2026-10-26", price: 2200.0 },
  ],
};

describe("McciCard", () => {
  it("每条序列一行: 品种、标准品指标、最新值、日涨跌、日期、一年高低", async () => {
    vi.useFakeTimers({ now: new Date("2026-10-27T20:00:00+08:00"), toFake: ["Date"] });
    mocks.fetchMcci.mockResolvedValue([fat]);
    render(<McciCard />);
    expect(await screen.findByText("肥煤")).toBeTruthy();
    expect(screen.getByText("A10 V30 S1.8 G90 Mt10")).toBeTruthy();
    // 界面不标数据来源.
    expect(screen.getByText("炼焦煤价格指数")).toBeTruthy();
    expect(document.body.textContent).not.toContain("钢铁网");
    expect(screen.getByText("2200")).toBeTruthy();
    expect(screen.getByText("▲+0.52%")).toBeTruthy();
    expect(screen.getByText("10-26")).toBeTruthy();
    expect(screen.getByText("低 2186 · 高 2200")).toBeTruthy();
    expect(screen.queryByText(/未更新/)).toBeNull();
  });

  it("走势悬停显示那天的日期与值", async () => {
    mocks.fetchMcci.mockResolvedValue([fat]);
    render(<McciCard />);
    const chart = await screen.findByRole("img", { name: /肥煤 近一年走势/ });
    // jsdom 没有布局: 按 240px 宽 (viewBox 的 2 倍) 模拟, 两端各留 8px 白边.
    chart.getBoundingClientRect = () => ({ left: 0, width: 240 }) as DOMRect;
    fireEvent.pointerMove(chart, { clientX: 0 });
    expect(screen.getByText("10-19 · 2186")).toBeTruthy();
    fireEvent.pointerMove(chart, { clientX: 120 });
    expect(screen.getByText("10-23 · 2189")).toBeTruthy();
    fireEvent.pointerMove(chart, { clientX: 236 });
    expect(screen.getByText("10-26 · 2200")).toBeTruthy();
    fireEvent.pointerLeave(chart);
    expect(screen.getByText("低 2186 · 高 2200")).toBeTruthy();
  });

  it("只有一个点时不说没拉到, 说数据不足", async () => {
    mocks.fetchMcci.mockResolvedValue([{ ...fat, points: fat.points.slice(-1) }]);
    render(<McciCard />);
    expect(await screen.findByText("数据不足")).toBeTruthy();
    expect(screen.queryByText("还没拉到数据")).toBeNull();
  });

  it("停更与还没拉到数据都要提示", async () => {
    vi.useFakeTimers({ now: new Date("2026-11-05T20:00:00+08:00"), toFake: ["Date"] });
    mocks.fetchMcci.mockResolvedValue([
      fat,
      { ...fat, code: "LJM_QM", type_name: "气煤", name: "气煤指数", points: [] },
    ]);
    render(<McciCard />);
    expect(await screen.findByText("已 10 天未更新")).toBeTruthy();
    expect(screen.getByText("还没拉到数据")).toBeTruthy();
  });

  it("接口取不到时整张卡不显示", async () => {
    mocks.fetchMcci.mockResolvedValue([]);
    const { container } = render(<McciCard />);
    await Promise.resolve();
    expect(container.textContent).toBe("");
  });
});
