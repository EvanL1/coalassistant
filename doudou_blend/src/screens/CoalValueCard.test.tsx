// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CoalValueCard } from "./CoalValueCard";
import type { BlendResult, CoalValue } from "../types";

function value(name: string, fob: number, fair: number, extra: Partial<CoalValue> = {}): CoalValue {
  return {
    name,
    fob,
    adjustment: fair - 2000,
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
          price_base: 2166.65,
          price_base_from_group: true,
        })}
      />,
    );
    expect(screen.getByText("煤价对比 (按质量)")).toBeTruthy();
    expect(screen.getByText("新星")).toBeTruthy();
    expect(screen.getByText("2100")).toBeTruthy();
    expect(screen.getByText("2054")).toBeTruthy();
    expect(screen.getByText("偏贵 46")).toBeTruthy();
    expect(screen.getByText("划算 12")).toBeTruthy();
    expect(screen.getByText(/基准品价 2167 元\/吨 \(由本次煤池反推\)/)).toBeTruthy();
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
          price_base: 2000,
          price_base_from_group: false,
        })}
      />,
    );
    expect(screen.getByText(/基准品价 2000 元\/吨 \(指定\)/)).toBeTruthy();
    expect(screen.getByText("外推")).toBeTruthy();
    expect(screen.getByText("缺 胶质、焦炭强度")).toBeTruthy();
  });
});
