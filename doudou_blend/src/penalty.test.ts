import { describe, expect, it } from "vitest";
import { mergePurchaseTerms, tierRate } from "./penalty";
import type { PenaltyTemplate } from "./penalty";

describe("tierRate", () => {
  it("把合同原文的每 0.1% 扣 8 元换算成 80 元/吨·%", () => {
    expect(tierRate(0.1, 8)).toBeCloseTo(80, 9);
  });

  it("每 0.01% 扣 2 元换算成 200 元/吨·%", () => {
    expect(tierRate(0.01, 2)).toBeCloseTo(200, 9);
  });

  it("每 1 点扣 5 元保持 5", () => {
    expect(tierRate(1, 5)).toBeCloseTo(5, 9);
  });

  it("步长非法时返回 0 而不是 Infinity", () => {
    expect(tierRate(0, 8)).toBe(0);
    expect(tierRate(-1, 8)).toBe(0);
    expect(tierRate(Number.NaN, 8)).toBe(0);
  });
});

const template: PenaltyTemplate = {
  contract_moisture: 8,
  moisture_excess_double_threshold: 12,
  clauses: [
    {
      indicator: "A",
      direction: "Upper",
      penalty: { tiers: [{ rate: 80 }], reject: 12 },
    },
    {
      indicator: "G",
      direction: "Lower",
      penalty: { tiers: [{ rate: 5 }], reject: 80 },
    },
  ],
};

describe("mergePurchaseTerms", () => {
  it("没有保证值的指标不产出条款", () => {
    const terms = mergePurchaseTerms(template, undefined, {});
    expect(terms).toBeNull();
  });

  it("按保证值从模板生成条款", () => {
    const terms = mergePurchaseTerms(template, undefined, { A: 10, G: 85 });
    expect(terms?.clauses).toHaveLength(2);
    expect(terms?.contract_moisture).toBe(8);
    const ash = terms?.clauses.find((clause) => clause.indicator === "A");
    expect(ash?.guarantee).toBe(10);
    expect(ash?.penalty.tiers[0].rate).toBe(80);
  });

  it("单煤覆盖优先于模板", () => {
    const terms = mergePurchaseTerms(
      template,
      { clauses: [{ indicator: "A", direction: "Upper", penalty: { tiers: [{ rate: 120 }], reject: 11 } }] },
      { A: 10, G: 85 },
    );
    const ash = terms?.clauses.find((clause) => clause.indicator === "A");
    expect(ash?.penalty.tiers[0].rate).toBe(120);
    expect(ash?.penalty.reject).toBe(11);
    const cohesion = terms?.clauses.find((clause) => clause.indicator === "G");
    expect(cohesion?.penalty.tiers[0].rate).toBe(5);
  });
});
