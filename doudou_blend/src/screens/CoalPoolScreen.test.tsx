// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CoalPoolScreen } from "./CoalPoolScreen";
import { addUserCoal, setCoalPref } from "../storage";
import type { Coal, CoalMaster, CoalPriceSnapshot, CoalValue } from "../types";

const mocks = vi.hoisted(() => ({ loadMaster: vi.fn() }));
vi.mock("../master_loader", () => ({ loadMaster: mocks.loadMaster }));

const master: CoalMaster = {
  version: "fixture", updated_at: "2026-10-28", description: "测试煤库",
  default_contract: { name: "测试合同", specs: [] },
  coals: [
    { name: "共享煤", status: "verified", props: { S: 1, A: 10, Y: 20 } },
    { name: "停用煤", status: "archived", props: { S: 1.2 } },
  ],
};
const snapshot: CoalPriceSnapshot = {
  ok: true, day: "2026-10-28", prev_day: "2026-10-27", computed_at: "2026-10-28T09:00:03Z",
  prices: [
    { name: "共享煤", price: 2202.4, prev_price: 2190.1, points: 1000, benchmark: null, missing: [] },
    { name: "停用煤", price: 2180, points: 990, benchmark: null, missing: ["Y"] },
  ],
};

function value(name: string, fair: number): CoalValue {
  return { name, fair, points: 1000, adjustment: 0, fob: null, gap: null, ratio: null,
    parts: [], missing: [], extrapolated: [] };
}

function reply(data: unknown) {
  return new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json" } });
}

function card(name: string) {
  return screen.getByText(name).closest(".coal-card")!;
}

beforeEach(() => {
  localStorage.clear();
  mocks.loadMaster.mockReset().mockResolvedValue(master);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("煤池卡片估价", () => {
  it("正常煤与停用煤都显示快照，副标题带快照日，不发多余现算", async () => {
    const fetch = vi.fn().mockImplementation(() => Promise.resolve(reply(snapshot)));
    vi.stubGlobal("fetch", fetch);
    render(<CoalPoolScreen />);
    await waitFor(() => expect(card("共享煤").textContent).toContain("估价 2,202 元/吨  ▲12"));
    expect(card("停用煤").textContent).toContain("估价 2,180 元/吨 · 缺 Y");
    expect(document.querySelector(".page-subtitle")?.textContent).toContain("· 估价 10-28");
    expect(fetch.mock.calls.map(([url]) => url)).toEqual(["/api/coal-prices"]);
  });

  it("修改煤、新增煤和快照缺项合并为一次现算，包含停用煤", async () => {
    setCoalPref("共享煤", { props_override: { S: 1.5 } });
    addUserCoal({ name: "自建煤", status: "verified", props: { S: 1.4 } });
    const requests: Coal[][] = [];
    vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/api/coal-prices") return reply({ ...snapshot, prices: snapshot.prices.slice(0, 1) });
      const { coals } = JSON.parse(init!.body as string) as { coals: Coal[] };
      requests.push(coals);
      return reply({ ok: true, coal_values: coals.map((coal) => value(coal.name, 2180)) });
    }));
    render(<CoalPoolScreen />);
    // 停用煤只是快照里没有, 用户没改过: 现算但不标"按修改值估".
    await waitFor(() => expect(card("停用煤").textContent).toContain("估价 2,180 元/吨"));
    expect(card("停用煤").textContent).not.toContain("按修改值估");
    expect(card("共享煤").textContent).toContain("估价 2,180 元/吨 · 按修改值估");
    expect(card("自建煤").textContent).toContain("估价 2,180 元/吨 · 按修改值估");
    expect(card("共享煤").textContent).not.toContain("▲");
    expect(requests).toHaveLength(1);
    expect(requests[0].map((coal) => coal.name)).toEqual(["自建煤", "共享煤", "停用煤"]);
    expect(requests[0][1].props.S).toBe(1.5);
    expect(requests[0].every((coal) => coal.fob === 0 && coal.frt === 0)).toBe(true);
  });

  it("修改化验值后重算，旧请求晚返回不能覆盖新值，等待时不沿用旧估价", async () => {
    setCoalPref("共享煤", { props_override: { S: 1.1 } });
    const pending: ((response: Response) => void)[] = [];
    vi.stubGlobal("fetch", vi.fn((url: string) => url === "/api/coal-prices"
      ? Promise.resolve(reply(snapshot))
      : new Promise<Response>((resolve) => pending.push(resolve))));
    render(<CoalPoolScreen />);
    await waitFor(() => expect(pending).toHaveLength(1));
    act(() => setCoalPref("共享煤", { props_override: { S: 1.2 } }));
    await waitFor(() => expect(pending).toHaveLength(2));
    await act(async () => pending[1](reply({ ok: true, coal_values: [value("共享煤", 2300)] })));
    expect(card("共享煤").textContent).toContain("估价 2,300 元/吨 · 按修改值估");
    await act(async () => pending[0](reply({ ok: true, coal_values: [value("共享煤", 2100)] })));
    expect(card("共享煤").textContent).toContain("估价 2,300 元/吨 · 按修改值估");
    act(() => setCoalPref("共享煤", { props_override: { S: 1.3 } }));
    expect(card("共享煤").textContent).toContain("估价暂不可用");
    await waitFor(() => expect(pending).toHaveLength(3));
    await act(async () => pending[2](reply({ ok: false, reason: "失败" })));
    expect(card("共享煤").textContent).toContain("估价暂不可用");
  });

  it("离线只让估价不可用，煤池指标和操作照常", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("离线")));
    render(<CoalPoolScreen />);
    await waitFor(() => expect(card("共享煤").textContent).toContain("估价暂不可用"));
    expect(document.querySelector(".page-subtitle")?.textContent).not.toContain("· 估价");
    expect(card("共享煤").textContent).toContain("20");
    expect(screen.getByRole("button", { name: "全部启用" })).toBeTruthy();
  });

  it("重新进入煤池重新取快照", async () => {
    let current = snapshot;
    const fetch = vi.fn(async () => reply(current));
    vi.stubGlobal("fetch", fetch);
    const first = render(<CoalPoolScreen />);
    await waitFor(() => expect(card("共享煤").textContent).toContain("2,202"));
    first.unmount();
    current = { ...snapshot, day: "2026-10-29", prices: snapshot.prices.map((entry) => ({ ...entry, price: 2400 })) };
    render(<CoalPoolScreen />);
    await waitFor(() => expect(card("共享煤").textContent).toContain("2,400"));
    expect(document.querySelector(".page-subtitle")?.textContent).toContain("· 估价 10-29");
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});

describe("煤池卡片指标格", () => {
  it("缺值格不能带页面空状态的 empty 类, 否则被撑高", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => reply(snapshot)));
    render(<CoalPoolScreen />);
    await waitFor(() => expect(card("共享煤")).toBeTruthy());
    const cell = [...card("共享煤").querySelectorAll(".coal-prop-label")]
      .find((label) => label.textContent === "挥发")!.parentElement!;
    expect(cell.textContent).toContain("—");
    expect(cell.classList.contains("empty")).toBe(false);
    expect(cell.classList.contains("coal-prop-empty")).toBe(true);
  });
});

describe("煤池估价走势", () => {
  it("点共享煤的估价行打开走势弹层, 不打开编辑器; Esc 关闭; 自建煤不可点", async () => {
    addUserCoal({ name: "自建煤", status: "verified", props: { S: 1.4 } });
    const history = { ok: true, name: "共享煤", points: [
      { day: "2026-10-27", price: 2190.1, benchmark: "基准", source: "backfill" },
      { day: "2026-10-28", price: 2202.4, benchmark: "基准", source: "live" },
    ] };
    vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/api/coal-prices") return reply(snapshot);
      if (url.startsWith("/api/coal-prices/history?")) return reply(history);
      const { coals } = JSON.parse(init!.body as string) as { coals: Coal[] };
      return reply({ ok: true, coal_values: coals.map((coal) => value(coal.name, 2180)) });
    }));
    render(<CoalPoolScreen />);
    const open = await screen.findByRole("button", { name: "查看共享煤估价走势" });
    await waitFor(() => expect(card("自建煤").textContent).toContain("估价 2,180 元/吨"));
    expect(screen.queryByRole("button", { name: "查看自建煤估价走势" })).toBeNull();
    fireEvent.click(open);
    expect(await screen.findByRole("dialog", { name: "共享煤" })).toBeTruthy();
    expect(screen.queryByLabelText(/保证值/)).toBeNull();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
