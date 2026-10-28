// @vitest-environment jsdom

import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CoalPriceLine } from "./CoalPriceLine";
import type { CardPrice } from "./coalPrices";

afterEach(cleanup);

describe("煤池估价行", () => {
  it.each([
    [12.3, "▲12"], [-8.1, "▼8"], [0, "持平"], [0.1, "持平"], [null, ""],
  ])("取整、千分位并显示涨跌 %s", (change, suffix) => {
    const { container } = render(<CoalPriceLine price={{
      kind: "snapshot", price: 2202.4, change: change as number | null, missing: [],
    }} />);
    expect(container.textContent).toBe(`估价 2,202 元/吨${suffix ? `  ${suffix}` : ""}`);
    expect(container.innerHTML).not.toMatch(/c-success|c-danger/);
  });

  it.each<[CardPrice, string]>([
    [{ kind: "live", price: 2179.8, missing: [], modified: true }, "估价 2,180 元/吨 · 按修改值估"],
    [{ kind: "live", price: 2180, missing: ["Y", "M"], modified: true }, "估价 2,180 元/吨 · 按修改值估 · 缺 Y、M"],
    [{ kind: "live", price: 2180, missing: [], modified: false }, "估价 2,180 元/吨"],
    [{ kind: "snapshot", price: 2180, change: null, missing: ["Y"] }, "估价 2,180 元/吨 · 缺 Y"],
    [{ kind: "unavailable" }, "估价暂不可用"],
  ])("显示现算、缺项或不可用状态 %#", (price, expected) => {
    const { container } = render(<CoalPriceLine price={price} />);
    expect(container.textContent).toBe(expected);
  });
});
