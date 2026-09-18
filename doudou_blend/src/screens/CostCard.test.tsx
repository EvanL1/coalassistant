// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CostCard } from "./CostCard";
import { INDICATOR_LABEL } from "../types";
import type { CostBreakdown } from "../types";

const base: CostBreakdown = {
  fob_per_ton: 1000,
  frt_per_ton: 100,
  cif_per_ton: 1100,
  total_fob: null,
  total_frt: null,
  total_cif: null,
  purchase_adjust_per_ton: 0,
  penalty_per_ton: 0,
  net_per_ton: 1100,
  total_purchase_adjust: null,
  total_penalty: null,
  total_net: null,
};

afterEach(() => {
  cleanup();
});

describe("CostCard", () => {
  it("无扣款时大字号标签是最低到厂价, 只显示大字号, 不渲染任何明细行", () => {
    render(<CostCard cost={base} />);
    expect(screen.getByTestId("cost-headline").textContent).toBe(
      "1100.00元/吨",
    );
    expect(screen.getByText("最低到厂价")).toBeTruthy();
    expect(screen.queryByText("最低净成本")).toBeNull();
    expect(screen.queryByText("到厂价")).toBeNull();
    expect(screen.queryByText("预计扣款")).toBeNull();
    expect(screen.queryByText("买入修正")).toBeNull();
  });

  it("有卖出扣款时大字号标签变成最低净成本, 显示扣款行与到厂价明细行", () => {
    render(<CostCard cost={{ ...base, penalty_per_ton: 64, net_per_ton: 1164 }} />);
    expect(screen.getByText("最低净成本")).toBeTruthy();
    expect(screen.queryByText("最低到厂价")).toBeNull();
    expect(screen.getByText("预计扣款")).toBeTruthy();
    expect(screen.getByText("64.00 元/吨")).toBeTruthy();
    expect(screen.getByText("到厂价")).toBeTruthy();
    expect(screen.getByTestId("cost-cif").textContent).toBe("1100.00 元/吨");
    // 净成本是大字号主位 (两段式 .cost-int/.cost-dec), 不是一个整串文本节点.
    expect(screen.getByText("1164", { selector: ".cost-int" })).toBeTruthy();
    expect(screen.getByText(".00", { selector: ".cost-dec" })).toBeTruthy();
  });

  it("有买入修正时大字号标签也变成最低净成本, 显示修正行与到厂价明细行, 折扣为负值", () => {
    render(
      <CostCard cost={{ ...base, purchase_adjust_per_ton: -40, net_per_ton: 1060 }} />,
    );
    expect(screen.getByText("最低净成本")).toBeTruthy();
    expect(screen.getByText("买入修正")).toBeTruthy();
    expect(screen.getByText("-40.00 元/吨")).toBeTruthy();
    expect(screen.getByText("到厂价")).toBeTruthy();
  });

  it("大字号主位显示净成本而非到厂价 (LP 按净成本求最优, 报价只是展示)", () => {
    render(<CostCard cost={{ ...base, penalty_per_ton: 30, net_per_ton: 1130 }} />);
    // cif_per_ton=1100, net_per_ton=1130 —— 主位必须是 1130, 不是 1100.
    expect(screen.getByText("1130", { selector: ".cost-int" })).toBeTruthy();
    expect(screen.queryByText("1100", { selector: ".cost-int" })).toBeNull();
    // 到厂价明细行仍展示报价原值.
    expect(screen.getByTestId("cost-cif").textContent).toBe("1100.00 元/吨");
  });

  it("买入修正与卖出扣款字段缺失 (老记录) 时按无扣款处理, 标签仍是最低到厂价, 不渲染明细行", () => {
    const legacy: CostBreakdown = {
      fob_per_ton: 900,
      frt_per_ton: 100,
      cif_per_ton: 1000,
    };
    render(<CostCard cost={legacy} />);
    expect(screen.getByText("最低到厂价")).toBeTruthy();
    expect(screen.queryByText("到厂价")).toBeNull();
    expect(screen.queryByText("预计扣款")).toBeNull();
    expect(screen.queryByText("买入修正")).toBeNull();
    // net_per_ton 也缺失 → 大字号回退展示 cif_per_ton, 不是伪造的 0.00
    expect(screen.getByText("1000", { selector: ".cost-int" })).toBeTruthy();
  });

  it("没有孤儿保证值时不显示模板缺失告警", () => {
    render(<CostCard cost={base} />);
    expect(screen.queryByText(/采购扣款模板/)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("有孤儿保证值时显示告警, 点名煤种与指标", () => {
    render(
      <CostCard
        cost={base}
        orphanedGuarantees={[
          { coal: "山西主焦", indicators: ["S", "A"] },
          { coal: "低硫肥煤", indicators: ["G"] },
        ]}
      />,
    );
    const warning = screen.getByRole("alert");
    expect(warning.textContent).toContain("采购扣款模板");
    expect(warning.textContent).toContain("山西主焦");
    expect(warning.textContent).toContain(INDICATOR_LABEL.S);
    expect(warning.textContent).toContain(INDICATOR_LABEL.A);
    expect(warning.textContent).toContain("低硫肥煤");
    expect(warning.textContent).toContain(INDICATOR_LABEL.G);
  });
});
