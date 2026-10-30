// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CoalPriceDialog } from "./CoalPriceDialog";
import type { CoalPriceHistory } from "./types";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

function serve(body: unknown, status = 200) {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify(body), { status })));
}

const history: CoalPriceHistory = {
  ok: true, name: "临北", points: [
    { day: "2026-10-13", price: 2150, benchmark: "低硫主焦煤指数", source: "backfill" },
    { day: "2026-10-14", price: 2160, benchmark: "低硫主焦煤指数", source: "backfill" },
    { day: "2026-10-15", price: 2190, benchmark: "安泽低硫主焦", source: "backfill" },
    { day: "2026-10-29", price: 2214, benchmark: "安泽低硫主焦", source: "live" },
  ],
};

describe("煤价走势弹层", () => {
  it("显示最新价、对标基准、区间高低, 基准变更处有标记, 有回算说明", async () => {
    serve(history);
    const { container } = render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    expect(await screen.findByText("2,214")).toBeTruthy();
    expect(screen.getByText("对标 安泽低硫主焦")).toBeTruthy();
    expect(container.textContent).toContain("最高 2,214");
    expect(container.textContent).toContain("最低 2,150");
    expect(container.querySelectorAll(".price-chart-marker")).toHaveLength(1);
    expect(container.textContent).toContain("虚线为回算");
    expect(container.textContent).not.toContain("不含你的修改");
  });

  it("改过化验值的煤注明历史口径", async () => {
    serve(history);
    const { container } = render(<CoalPriceDialog name="临北" modified onClose={() => {}} />);
    await screen.findByText("2,214");
    expect(container.textContent).toContain("历史为共享煤库口径, 不含你的修改");
  });

  it.each<[string, () => void, string]>([
    ["取不到", () => serve({}, 503), "走势暂时取不到"],
    ["不足两天", () => serve({ ...history, points: history.points.slice(-1) }), "数据不足两天, 画不出走势"],
  ])("%s", async (_, setup, text) => {
    setup();
    render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    expect(await screen.findByText(text)).toBeTruthy();
  });

  it("请求回来前显示加载中", () => {
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise(() => {})));
    render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    expect(screen.getByText("加载中…")).toBeTruthy();
  });
});

describe("煤价走势的范围与请求切换", () => {
  it("所选范围不足两天时仍能切回一年", async () => {
    serve({ ...history, points: [
      { ...history.points[0], day: "2026-01-01" }, history.points[3],
    ] });
    render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    await screen.findByText("2,214");
    fireEvent.click(screen.getByRole("button", { name: "1月" }));
    expect(screen.getByText("数据不足两天, 画不出走势")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "1年" }));
    expect(screen.getByRole("img")).toBeTruthy();
  });

  it("切换煤名时清除旧走势, 新请求完成前显示加载中", async () => {
    let reply!: (response: Response) => void;
    const pending = new Promise<Response>((resolve) => { reply = resolve; });
    vi.stubGlobal("fetch", vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(history)))
      .mockReturnValueOnce(pending));
    const { rerender } = render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    await screen.findByText("2,214");
    rerender(<CoalPriceDialog name="另煤" modified={false} onClose={() => {}} />);
    expect(screen.getByText("加载中…")).toBeTruthy();
    expect(screen.queryByText("2,214")).toBeNull();
    await act(async () => { reply(new Response(JSON.stringify({ ...history, name: "另煤" }))); });
    expect(await screen.findByText("2,214")).toBeTruthy();
  });

  it("旧煤请求后到也不能覆盖新煤走势", async () => {
    let reply!: (response: Response) => void;
    const pending = new Promise<Response>((resolve) => { reply = resolve; });
    vi.stubGlobal("fetch", vi.fn().mockReturnValueOnce(pending)
      .mockResolvedValueOnce(new Response(JSON.stringify({ ...history, name: "另煤",
        points: history.points.map((point) => ({ ...point, price: 3000 })),
      }))));
    const { rerender } = render(<CoalPriceDialog name="临北" modified={false} onClose={() => {}} />);
    rerender(<CoalPriceDialog name="另煤" modified={false} onClose={() => {}} />);
    await screen.findByText("3,000");
    await act(async () => { reply(new Response(JSON.stringify(history))); });
    expect(screen.queryByText("2,214")).toBeNull();
    expect(screen.getByText("3,000")).toBeTruthy();
  });
});
