// @vitest-environment jsdom

import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { PriceChart, type ChartPoint } from "./PriceChart";

afterEach(cleanup);

const plain: ChartPoint[] = [
  { date: "2026-10-27", price: 2200 }, { date: "2026-10-28", price: 2210 }, { date: "2026-10-29", price: 2214 },
];

describe("走势图", () => {
  it("不传新参数时只有一条实线, 没有基准标记", () => {
    const { container } = render(<PriceChart label="测试" points={plain} />);
    const lines = container.querySelectorAll(".index-chart-line");
    expect(lines).toHaveLength(1);
    expect(lines[0].getAttribute("stroke-dasharray")).toBeNull();
    expect(container.querySelectorAll(".price-chart-marker")).toHaveLength(0);
  });

  it("回算段画虚线, 与实线段共用交界点", () => {
    const points: ChartPoint[] = [
      { date: "2026-10-27", price: 2200, dashed: true },
      { date: "2026-10-28", price: 2210, dashed: true },
      { date: "2026-10-29", price: 2214 },
      { date: "2026-10-30", price: 2220 },
    ];
    const { container } = render(<PriceChart label="测试" points={points} />);
    const lines = [...container.querySelectorAll(".index-chart-line")];
    expect(lines.map((line) => line.getAttribute("stroke-dasharray"))).toEqual(["4 3", null]);
    const coords = lines.map((line) => line.getAttribute("points")!.split(" "));
    expect(coords[0]).toHaveLength(3);
    expect(coords[1]).toHaveLength(2);
    expect(coords[0][2]).toBe(coords[1][0]);
  });

  it("基准变更处画竖标; tooltip 显示附加行", () => {
    const points: ChartPoint[] = plain.map((point, i) => ({ ...point, notes: [i < 2 ? "对标 甲" : "对标 乙", "回算"] }));
    const { container, getByRole } = render(<PriceChart label="测试" points={points} markers={[2]} />);
    expect(container.querySelectorAll(".price-chart-marker")).toHaveLength(1);
    const svg = container.querySelector("svg")!;
    fireEvent.keyDown(svg, { key: "End" });
    expect(getByRole("status").textContent).toContain("对标 乙");
    expect(getByRole("status").textContent).toContain("回算");
  });
});
