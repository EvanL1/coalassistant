// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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

/** 2025-10-27 起每个工作日一个点, 到 2026-10-26, 价格逐日 +1. */
function yearOfPoints(): [string, number][] {
  const out: [string, number][] = [];
  const day = new Date("2025-10-27T00:00:00Z");
  const end = new Date("2026-10-26T00:00:00Z");
  let price = 2000;
  while (day <= end) {
    const weekday = day.getUTCDay();
    if (weekday !== 0 && weekday !== 6) out.push([day.toISOString().slice(0, 10), price++]);
    day.setUTCDate(day.getUTCDate() + 1);
  }
  return out;
}

const drawnPoints = () =>
  (document.querySelector(".index-chart-line")?.getAttribute("points") ?? "")
    .split(" ")
    .filter(Boolean).length;

describe("指数走势弹层", () => {
  async function openLowSulfur() {
    vi.useFakeTimers({ now: new Date("2026-10-27T20:00:00+08:00"), toFake: ["Date"] });
    const points = yearOfPoints();
    mocks.fetchMcci.mockResolvedValue([
      { ...series("低硫主焦煤", points), props: { A: 9.5, V: 25, S: 0.6, G: 85, M: 10 } },
    ]);
    render(<IndexTicker />);
    const opener = await screen.findByRole("button", { name: /低硫主焦煤 最新 \d+, 查看走势/ });
    opener.focus();
    fireEvent.click(opener);
    return { opener, points };
  }

  it("点开显示品种、标准品规格、最新价与日/周涨跌; 不写数据来源", async () => {
    const { points } = await openLowSulfur();
    const dialog = screen.getByRole("dialog", { name: "低硫主焦煤" });
    expect(dialog.textContent).toContain("标准品 A9.5 V25 S0.6 G85 Mt10");
    expect(dialog.textContent).toContain(String(points[points.length - 1][1]));
    expect(dialog.textContent).toContain("2026-10-26");
    expect(dialog.textContent).toMatch(/日 ▲\+0\.04%/);
    expect(dialog.textContent).toMatch(/周 ▲\+/);
    expect(document.body.textContent).not.toMatch(/钢铁网|mysteel/i);
    // 焦点进弹层 (关闭按钮)
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "关闭" }));
  });

  it("时间范围切换改变画出的点数, 默认一年", async () => {
    const { points } = await openLowSulfur();
    expect(drawnPoints()).toBe(points.length);
    fireEvent.click(screen.getByRole("button", { name: "1月" }));
    expect(screen.getByRole("button", { name: "1月" }).getAttribute("aria-pressed")).toBe("true");
    // 09-27 之后的工作日: 9/28 … 10/26 共 21 天
    expect(drawnPoints()).toBe(21);
    fireEvent.click(screen.getByRole("button", { name: "3月" }));
    expect(drawnPoints()).toBeGreaterThan(21);
    expect(drawnPoints()).toBeLessThan(points.length);
  });

  it("键盘逐日查看: tooltip 显示日期与价格", async () => {
    const { points } = await openLowSulfur();
    const chart = screen.getByRole("img", { name: /低硫主焦煤走势/ });
    fireEvent.keyDown(chart, { key: "ArrowLeft" });
    const [date, price] = points[points.length - 2];
    const tip = screen.getByRole("status");
    expect(tip.textContent).toContain(date);
    expect(tip.textContent).toContain(String(price));
    fireEvent.keyDown(chart, { key: "Home" });
    expect(screen.getByRole("status").textContent).toContain(points[0][0]);
  });

  it("Esc 或点遮罩关闭, 焦点回到行情条那一项", async () => {
    const { opener } = await openLowSulfur();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);

    fireEvent.click(opener);
    // 点弹层内部不关
    fireEvent.click(screen.getByRole("dialog"));
    expect(screen.getByRole("dialog")).toBeTruthy();
    fireEvent.click(document.querySelector(".modal-backdrop")!);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
