// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PenaltyTemplateEditor } from "./PenaltyTemplateEditor";
import { getPenaltyTemplate, setPenaltyTemplate } from "./penaltyStorage";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

beforeEach(() => {
  localStorage.clear();
  vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
});

/** 照抄采购合同"灰分保证 ≤10%（保证值在煤卡里填），每超 0.1% 扣 8 元/吨，超 12% 拒收". */
function fillAshClause({ amount = "8", reject = "12" } = {}) {
  fireEvent.click(screen.getByText(/加一条条款/));
  fireEvent.change(screen.getByLabelText(/第 1 条条款的指标/), {
    target: { value: "A" },
  });
  fireEvent.change(screen.getByLabelText(/每多少/), { target: { value: "0.1" } });
  fireEvent.change(screen.getByLabelText(/档扣款/), { target: { value: amount } });
  fireEvent.change(screen.getByLabelText(/拒收线/), { target: { value: reject } });
}

describe("采购扣款模板", () => {
  it("照抄的每 0.1% 扣 8 元, 存下去是 80 元/吨·%", () => {
    render(<PenaltyTemplateEditor />);
    fillAshClause();
    fireEvent.click(screen.getByText("保存模板"));
    expect(getPenaltyTemplate()?.clauses).toEqual([
      {
        indicator: "A",
        direction: "Upper",
        penalty: { tiers: [{ rate: 80 }], reject: 12 },
      },
    ]);
  });

  it("扣款金额留空时报错并挡住保存, 不会静默存成 0 元/吨", () => {
    render(<PenaltyTemplateEditor />);
    fillAshClause({ amount: "" });
    expect(screen.getByText(/「扣」要填/)).toBeTruthy();
    fireEvent.click(screen.getByText("保存模板"));
    expect(getPenaltyTemplate()).toBeNull();
  });

  it("同一指标两条条款时报错并挡住保存", () => {
    render(<PenaltyTemplateEditor />);
    fillAshClause();
    fireEvent.click(screen.getByText(/加一条条款/));
    fireEvent.change(screen.getByLabelText(/第 2 条条款的指标/), {
      target: { value: "A" },
    });
    fireEvent.change(screen.getAllByLabelText(/每多少/)[1], {
      target: { value: "0.1" },
    });
    fireEvent.change(screen.getAllByLabelText(/档扣款/)[1], {
      target: { value: "9" },
    });
    fireEvent.change(screen.getAllByLabelText(/拒收线/)[1], {
      target: { value: "12" },
    });
    fireEvent.click(screen.getByText("保存模板"));
    expect(getPenaltyTemplate()).toBeNull();
    expect(screen.getByText(/两条条款/)).toBeTruthy();
  });

  it("合同水分一并存下来", () => {
    render(<PenaltyTemplateEditor />);
    fillAshClause();
    fireEvent.change(screen.getByLabelText(/合同水分/), { target: { value: "8" } });
    fireEvent.click(screen.getByText("保存模板"));
    expect(getPenaltyTemplate()?.contract_moisture).toBe(8);
  });

  it("已存的模板回显出来, 不是空白", () => {
    setPenaltyTemplate({
      clauses: [
        {
          indicator: "A",
          direction: "Upper",
          penalty: { tiers: [{ rate: 80 }], reject: 12 },
        },
      ],
      contract_moisture: 8,
    });
    render(<PenaltyTemplateEditor />);
    expect((screen.getByLabelText(/档扣款/) as HTMLInputElement).value).toBe("80");
    expect((screen.getByLabelText(/拒收线/) as HTMLInputElement).value).toBe("12");
    expect((screen.getByLabelText(/合同水分/) as HTMLInputElement).value).toBe("8");
  });

  it("清空模板后本地不再留条款", () => {
    setPenaltyTemplate({
      clauses: [
        {
          indicator: "A",
          direction: "Upper",
          penalty: { tiers: [{ rate: 80 }], reject: 12 },
        },
      ],
    });
    render(<PenaltyTemplateEditor />);
    fireEvent.click(screen.getByText("清空模板"));
    expect(getPenaltyTemplate()).toBeNull();
  });
});
