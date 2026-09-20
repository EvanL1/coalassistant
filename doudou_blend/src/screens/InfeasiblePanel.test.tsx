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
            margin: 0,
            achievable: 11.3125,
            relax_to: 11.4,
          },
          {
            indicator: "G",
            label_zh: "粘结",
            direction: "Lower",
            required: 85,
            enforced: 85,
            margin: 0,
            achievable: 78,
            relax_to: 77.9,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "11.31", "≤11.40"]);
    expect(rowCells("粘结")).toEqual(["粘结", "≥85.00", "78.00", "≥77.90"]);
  });

  /**
   * 安全余量把执行界收得比合同界紧: 合同 ≤9 看着已经达标 (最好能做到 9.00),
   * 真正卡住的是 ≤8 那条线. 不说破的话, 这一行读起来就是工具在自相矛盾;
   * 差值也必须按执行界算, 否则是 0.00, 用户不知道该放宽多少.
   */
  it("安全余量收紧执行界时说破那条线, 并给出该填的数", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "A",
            label_zh: "灰",
            direction: "Upper",
            required: 9,
            enforced: 8,
            margin: 1,
            achievable: 9,
            relax_to: 10,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual([
      "灰",
      "≤9.00含 1 安全余量, 按 ≤8.00 执行",
      "9.00",
      "≤10.00",
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
            margin: 0,
            achievable: 11.3125,
            relax_to: 11.4,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "11.31", "≤11.40"]);
    expect(screen.queryByText(/按 ≤10.10 执行/)).toBeNull();
  });

  /**
   * 截断判定在下限一侧会把 ≥14.95 抬成按 ≥15.0 执行 —— 收紧了, 但一点安全余量都没设.
   * 这时印"含安全余量"是给用户编一个不存在的原因, 成因只能看 margin, 不能按方向猜.
   */
  it("判定规则收紧执行界时按成因说话, 不冒充安全余量", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "Y",
            label_zh: "胶质",
            direction: "Lower",
            required: 14.95,
            enforced: 15,
            margin: 0,
            achievable: 14.98,
            relax_to: 14.9,
          },
        ])}
      />,
    );

    expect(rowCells("胶质")).toEqual([
      "胶质",
      "≥14.95按合同判定规则, 按 ≥15.00 执行",
      "14.98",
      "≥14.90",
    ]);
    expect(screen.queryByText(/安全余量/)).toBeNull();
  });

  /**
   * 放宽这一项必要但不充分时 core 给 null (譬如 LP 通了却卡在岩相精确复核).
   * 这一格只能留白, 那句"改成这个数就能求出配方"也不能对它说 —— 没有那个数.
   */
  it("试不出可填的数时留白, 不承诺某个数管用", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "A",
            label_zh: "灰",
            direction: "Upper",
            required: 10,
            enforced: 10.0999,
            margin: 0,
            achievable: 12,
            relax_to: null,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "12.00", "—"]);
    expect(screen.queryByText(/就能求出配方/)).toBeNull();
    expect(screen.getByText(/只能确定非放宽它不可/)).toBeTruthy();
  });

  // 混着来: 有数的那几项照旧承诺, 没数的那几项不跟着被承诺.
  it("一部分试得出一部分试不出时, 两句话各管各的", () => {
    render(
      <InfeasiblePanel
        result={infeasible([
          {
            indicator: "A",
            label_zh: "灰",
            direction: "Upper",
            required: 10,
            enforced: 10,
            margin: 0,
            achievable: 11.3125,
            relax_to: 11.4,
          },
          {
            indicator: "S",
            label_zh: "硫",
            direction: "Upper",
            required: 1,
            enforced: 1,
            margin: 0,
            achievable: 1.4,
            relax_to: null,
          },
        ])}
      />,
    );

    expect(rowCells("灰")).toEqual(["灰", "≤10.00", "11.31", "≤11.40"]);
    expect(rowCells("硫")).toEqual(["硫", "≤1.00", "1.40", "—"]);
    expect(screen.getByText(/就能求出配方/)).toBeTruthy();
    expect(screen.getByText(/只能确定非放宽它不可/)).toBeTruthy();
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
