// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CoalValueCard } from "./CoalValueCard";
import type { BlendResult, CoalValue } from "../types";

function value(
  name: string,
  fob: number,
  fair: number,
  extra: Partial<CoalValue> = {},
): CoalValue {
  return {
    name,
    fob,
    adjustment: fair / 2 - 1000,
    points: fair / 2,
    fair,
    gap: fob - fair,
    ratio: fob / fair,
    parts: [],
    missing: [],
    extrapolated: [],
    ...extra,
  };
}

function result(extra: Partial<BlendResult>): BlendResult {
  return {
    ok: true,
    recipe: {},
    orders: [],
    indicator_check: [],
    warnings: [],
    ...extra,
  };
}

afterEach(() => {
  cleanup();
});

describe("CoalValueCard", () => {
  it("没有 coal_values 时不显示", () => {
    const { container } = render(<CoalValueCard result={result({})} />);
    expect(container.innerHTML).toBe("");
    render(<CoalValueCard result={result({ coal_values: [] })} />);
    expect(screen.queryByText("煤价对比 (按质量)")).toBeNull();
  });

  it("逐煤显示报价/应值/差额, 正差额为偏贵, 负差额为划算", () => {
    render(
      <CoalValueCard
        result={result({
          coal_values: [
            value("新星", 2100, 2053.65),
            value("荣欣", 2500, 2512.25),
          ],
          market_price: 2000,
          market_price_from_group: true,
        })}
      />,
    );
    expect(screen.getByText("煤价对比 (按质量)")).toBeTruthy();
    expect(screen.getByText("新星")).toBeTruthy();
    expect(screen.getByText("2100")).toBeTruthy();
    expect(screen.getByText("2054")).toBeTruthy();
    expect(screen.getByText("偏贵 46")).toBeTruthy();
    expect(screen.getByText("质量指数")).toBeTruthy();
    expect(screen.getByText("1027")).toBeTruthy();
    expect(screen.getByText("划算 12")).toBeTruthy();
    expect(
      screen.getByText(/市场价 2000 元\/吨 = 标准品 1000 点 \(由 2 种有报价的煤反推\)/),
    ).toBeTruthy();
    expect(screen.getByText(/只用出厂价, 不含运费/)).toBeTruthy();
  });

  it("指定基准价时注明来源; 外推与缺指标逐煤标出", () => {
    render(
      <CoalValueCard
        result={result({
          ok: false,
          coal_values: [
            value("北沟", 2200, 2165.45, { extrapolated: ["S"] }),
            value("浮精", 2000, 2068.65, { missing: ["Y", "CSR"] }),
          ],
          market_price: 2000,
          market_price_from_group: false,
        })}
      />,
    );
    expect(
      screen.getByText(/市场价 2000 元\/吨 = 标准品 1000 点 \(指定\)/),
    ).toBeTruthy();
    expect(screen.getByText("外推")).toBeTruthy();
    expect(screen.getByText("缺 胶质、焦炭强度")).toBeTruthy();
  });
});

describe("CoalValueCard 口径与标签", () => {
  const values = [
    value("北沟", 2200, 2165.45, { extrapolated: ["S"] }),
    value("浮精", 2000, 2000.3),
    value("新星", 2100, 2099.7),
  ];

  it("表头写出厂价 (求解口径); 有报价推算时脚注说明按推算价对比", () => {
    render(<CoalValueCard result={result({ coal_values: values })} drifted />);
    expect(screen.getByText("出厂价 (求解口径)")).toBeTruthy();
    expect(screen.queryByText("报价")).toBeNull();
    expect(screen.getByText(/按推算后的出厂价对比/)).toBeTruthy();
  });

  it("无推算时不出推算说明", () => {
    render(<CoalValueCard result={result({ coal_values: values })} />);
    expect(screen.queryByText(/按推算后的出厂价对比/)).toBeNull();
  });

  it("没报价的煤出厂价与差额显示 —, 照样给估算价; 按质量指数从高到低排", () => {
    render(
      <CoalValueCard
        result={result({
          coal_values: [
            value("新星", 2100, 2000),
            value("无价煤", 0, 2400, { fob: null, gap: null, ratio: null }),
          ],
          market_price: 2000,
          market_price_from_group: true,
        })}
      />,
    );
    const rows = screen.getAllByRole("row").slice(1);
    expect(rows.map((row) => row.firstElementChild?.textContent)).toEqual([
      "无价煤",
      "新星",
    ]);
    expect(rows[0].textContent).toContain("2400");
    expect(screen.getAllByText("—")).toHaveLength(2);
    expect(screen.getByText(/由 1 种有报价的煤反推/)).toBeTruthy();
    expect(screen.getByText(/未录出厂价的煤只给估算价/)).toBeTruthy();
  });

  it("取整后持平用中性色, 不按正负号着色", () => {
    render(<CoalValueCard result={result({ coal_values: values })} />);
    for (const cellEl of screen.getAllByText("持平")) {
      expect((cellEl as HTMLElement).style.color).toBe("var(--c-text-3)");
    }
  });

  it("外推悬停说明用中文指标名", () => {
    render(<CoalValueCard result={result({ coal_values: values })} />);
    expect(screen.getByText("外推").getAttribute("title")).toBe(
      "超出系数线性区间: 硫",
    );
  });
});
