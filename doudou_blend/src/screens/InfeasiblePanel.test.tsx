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
            achievable: 11.3125,
          },
          {
            indicator: "G",
            label_zh: "粘结",
            direction: "Lower",
            required: 85,
            achievable: 78,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "11.31", "1.31"]);
    expect(rowCells("粘结")).toEqual(["粘结", "≥85.00", "78.00", "7.00"]);
  });

  // 合同界本身够得到、只是被安全余量或判定规则收紧时, 差值是负的, 写出来会误导.
  it("合同界够得到时差值留空", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "A",
            label_zh: "灰",
            direction: "Upper",
            required: 10,
            achievable: 9.8,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "9.80", "—"]);
  });

  it("定位不到单项约束时直说, 不硬凑真凶", () => {
    render(<InfeasiblePanel result={infeasible([])} />);

    expect(screen.queryByRole("table")).toBeNull();
    expect(
      screen.getByText(/没有单独一项约束能解释这次不可行/),
    ).toBeTruthy();
    expect(screen.getByText("约束冲突, LP 不可行")).toBeTruthy();
  });

  // 诊断字段上线前存下来的结果不带这个字段, 面板不能因此崩掉.
  it("结果里没有 infeasible_bounds 字段时退回通用说明", () => {
    const legacy = infeasible([]);
    delete legacy.infeasible_bounds;
    render(<InfeasiblePanel result={legacy} />);

    expect(
      screen.getByText(/没有单独一项约束能解释这次不可行/),
    ).toBeTruthy();
  });
});
