// @vitest-environment jsdom

import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { BlendResult, InfeasibleBound } from "../types";
import { InfeasiblePanel } from "./InfeasiblePanel";

afterEach(() => {
  cleanup();
});

function infeasible(bounds: InfeasibleBound[]): BlendResult {
  return {
    ok: false,
    reason: "约束冲突, LP 不可行",
    infeasible_bounds: bounds,
    recipe: {},
    cost: null,
    orders: [],
    indicator_check: [],
    warnings: [],
    quality_status: "NeedsReview",
  };
}

/** 表体某一行的四个单元格文本. */
function rowCells(label: string): string[] {
  const cell = screen.getByText(label);
  const row = cell.closest("tr");
  if (!row) throw new Error(`${label} 不在表格行里`);
  return within(row)
    .getAllByRole("cell")
    .map((node) => node.textContent ?? "");
}

describe("InfeasiblePanel", () => {
  // 上下两个方向各一行: 只有一行时, 差值算反了也看不出来.
  it("上限与下限各按各的方向算差多少", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "A",
            label_zh: "灰",
            direction: "Upper",
            required: 10,
            enforced: 10,
            achievable: 11.3125,
          },
          {
            indicator: "G",
            label_zh: "粘结",
            direction: "Lower",
            required: 85,
            enforced: 85,
            achievable: 78,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "11.31", "1.31"]);
    expect(rowCells("粘结")).toEqual(["粘结", "≥85.00", "78.00", "7.00"]);
  });

  /**
   * 安全余量把执行界收得比合同界紧: 合同 ≤9 看着已经达标 (最好能做到 9.00),
   * 真正卡住的是 ≤8 那条线. 不说破的话, 这一行读起来就是工具在自相矛盾;
   * 差值也必须按执行界算, 否则是 0.00, 用户不知道该放宽多少.
   */
  it("安全余量收紧执行界时说破那条线, 差值按执行界算", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "A",
            label_zh: "灰",
            direction: "Upper",
            required: 9,
            enforced: 8,
            achievable: 9,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual([
      "灰",
      "≤9.00含安全余量, 按 ≤8.00 执行",
      "9.00",
      "1.00",
    ]);
  });

  // 截断判定把执行界放松 (≤10 实际按 ≤10.0999 判), 这一侧不必打扰用户:
  // 按合同界放宽同样有效, 多一行小字只是噪音.
  it("判定规则放宽执行界时不多话", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "A",
            label_zh: "灰",
            direction: "Upper",
            required: 10,
            enforced: 10.0999,
            achievable: 11.3125,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "11.31", "1.31"]);
    expect(screen.queryByText(/按 ≤10.10 执行/)).toBeNull();
  });

  it("定位不到单项约束时直说, 不硬凑真凶", () => {
    render(<InfeasiblePanel result={infeasible([])} />);

    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.getByText(/没有单独一项约束能解释这次不可行/)).toBeTruthy();
    expect(screen.getByText("约束冲突, LP 不可行")).toBeTruthy();
  });

  // 诊断字段上线前存下来的结果不带这个字段, 面板不能因此崩掉.
  it("结果里没有 infeasible_bounds 字段时退回通用说明", () => {
    const legacy = infeasible([]);
    delete legacy.infeasible_bounds;
    render(<InfeasiblePanel result={legacy} />);

    expect(screen.getByText(/没有单独一项约束能解释这次不可行/)).toBeTruthy();
  });

  /**
   * ok=true 却没有成本结构 (畸形结果/存量记录): 这不是合同不可行, 面板不能对着一个
   * 成功的求解喊"不可行", 更不能说"没有单独一项约束能解释".
   */
  it("结果不完整时换标题与说法, 不误报成不可行", () => {
    const incomplete: BlendResult = {
      ...infeasible([]),
      ok: true,
      reason: null,
    };
    render(<InfeasiblePanel result={incomplete} />);

    expect(screen.getByText("✗ 结果不完整")).toBeTruthy();
    expect(screen.getByText(/没有返回成本结构/)).toBeTruthy();
    expect(screen.queryByText(/没有单独一项约束能解释/)).toBeNull();
    expect(screen.queryByText("✗ 不可行")).toBeNull();
  });
});
