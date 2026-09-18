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
  it("无扣款时只显示到厂价与净成本", () => {
    render(<CostCard cost={base} />);
    expect(screen.getByText("到厂价")).toBeTruthy();
    expect(screen.queryByText("预计扣款")).toBeNull();
    expect(screen.queryByText("买入修正")).toBeNull();
  });

  it("有卖出扣款时显示扣款行", () => {
    render(<CostCard cost={{ ...base, penalty_per_ton: 64, net_per_ton: 1164 }} />);
    expect(screen.getByText("预计扣款")).toBeTruthy();
    expect(screen.getByText("64.00 元/吨")).toBeTruthy();
    expect(screen.getByText("1164.00 元/吨")).toBeTruthy();
  });

  it("有买入修正时显示修正行, 折扣为负值", () => {
    render(
      <CostCard cost={{ ...base, purchase_adjust_per_ton: -40, net_per_ton: 1060 }} />,
    );
    expect(screen.getByText("买入修正")).toBeTruthy();
    expect(screen.getByText("-40.00 元/吨")).toBeTruthy();
  });

  it("买入修正与卖出扣款字段缺失 (老记录) 时按无扣款处理, 不崩溃", () => {
    const legacy: CostBreakdown = {
      fob_per_ton: 900,
      frt_per_ton: 100,
      cif_per_ton: 1000,
    };
    render(<CostCard cost={legacy} />);
    expect(screen.getByText("到厂价")).toBeTruthy();
    expect(screen.queryByText("预计扣款")).toBeNull();
    expect(screen.queryByText("买入修正")).toBeNull();
    // net_per_ton 也缺失 → 回退展示 cif_per_ton, 不是伪造的 0.00
    expect(screen.getAllByText("1000.00 元/吨").length).toBeGreaterThan(0);
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
