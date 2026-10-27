// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IndexTicker } from "./IndexTicker";

const mocks = vi.hoisted(() => ({ fetchMcci: vi.fn() }));

vi.mock("./mcci", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./mcci")>()),
  fetchMcci: mocks.fetchMcci,
}));

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const series = (type_name: string, prices: [string, number][]) => ({
  code: type_name,
  type_name,
  name: `${type_name}指数`,
  coal_type: type_name,
  props: { S: 1 },
  points: prices.map(([date, price]) => ({ date, price })),
});

describe("IndexTicker", () => {
  it("行情条显示炼焦煤价格指数: 品种、最新值、日涨跌, 不再显示期货", async () => {
    vi.useFakeTimers({ now: new Date("2026-10-27T20:00:00+08:00"), toFake: ["Date"] });
    mocks.fetchMcci.mockResolvedValue([
      series("低硫主焦煤", [["2026-10-23", 2325.3], ["2026-10-26", 2321]]),
      series("肥煤", [["2026-10-23", 2188.7], ["2026-10-26", 2200]]),
    ]);
    render(<IndexTicker />);
    // 主焦三档名字太长, 去掉末尾的"煤".
    expect(await screen.findByText("低硫主焦")).toBeTruthy();
    expect(screen.getByText("肥煤")).toBeTruthy();
    expect(screen.getByText("2321")).toBeTruthy();
    expect(screen.getByText("▼0.18%")).toBeTruthy();
    expect(screen.getByText("▲+0.52%")).toBeTruthy();
    expect(screen.queryByText("焦炭")).toBeNull();
    // 不停更就不标日期.
    expect(screen.queryByText(/\(10-26\)/)).toBeNull();
  });

  it("停更时标出数据日期; 没数据的品种不显示, 全都没有时整条不显示", async () => {
    vi.useFakeTimers({ now: new Date("2026-11-05T20:00:00+08:00"), toFake: ["Date"] });
    mocks.fetchMcci.mockResolvedValue([
      series("肥煤", [["2026-10-23", 2188.7], ["2026-10-26", 2200]]),
      series("气煤", []),
    ]);
    render(<IndexTicker />);
    expect(await screen.findByText("(10-26)")).toBeTruthy();
    expect(screen.queryByText("气煤")).toBeNull();
    cleanup();

    mocks.fetchMcci.mockResolvedValue([]);
    const { container } = render(<IndexTicker />);
    await Promise.resolve();
    expect(container.textContent).toBe("");
  });
});
