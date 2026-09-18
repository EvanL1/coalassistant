import { describe, expect, it } from "vitest";
import { mergePurchaseTerms, tierRate } from "./penalty";
import type { PenaltyTemplate } from "./penalty";

describe("tierRate", () => {
  it("把合同原文的每 0.1% 扣 8 元换算成 80 元/吨·%", () => {
    expect(tierRate({ step: 0.1, amount: 8 })).toBeCloseTo(80, 9);
  });

  it("每 0.01% 扣 2 元换算成 200 元/吨·%", () => {
    expect(tierRate({ step: 0.01, amount: 2 })).toBeCloseTo(200, 9);
  });

  it("每 1 点扣 5 元保持 5", () => {
    expect(tierRate({ step: 1, amount: 5 })).toBeCloseTo(5, 9);
  });

  it("0 元是合法的零费率档位, 不是错误信号", () => {
    expect(tierRate({ step: 1, amount: 0 })).toBe(0);
  });

  it("step 非法(非正数/非有限)返回 null 而不是 Infinity", () => {
    expect(tierRate({ step: 0, amount: 8 })).toBeNull();
    expect(tierRate({ step: -1, amount: 8 })).toBeNull();
    expect(tierRate({ step: Number.NaN, amount: 8 })).toBeNull();
  });

  it("amount 非法(负数/非有限)返回 null —— 负费率等于扣款倒贴钱", () => {
    expect(tierRate({ step: 0.1, amount: -8 })).toBeNull();
    expect(tierRate({ step: 0.1, amount: Number.NaN })).toBeNull();
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
  it("没有保证值的指标不产出条款, 也不算孤儿", () => {
    const merged = mergePurchaseTerms(template, undefined, {});
    expect(merged.terms).toBeNull();
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("按保证值从模板生成条款, 水分字段一并带过来", () => {
    const merged = mergePurchaseTerms(template, undefined, { A: 10, G: 85 });
    expect(merged.terms?.clauses).toHaveLength(2);
    expect(merged.terms?.contract_moisture).toBe(8);
    expect(merged.terms?.moisture_excess_double_threshold).toBe(12);
    const ash = merged.terms?.clauses.find((clause) => clause.indicator === "A");
    expect(ash?.guarantee).toBe(10);
    expect(ash?.penalty.tiers[0].rate).toBe(80);
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("单煤覆盖优先于模板", () => {
    const merged = mergePurchaseTerms(
      template,
      { clauses: [{ indicator: "A", direction: "Upper", penalty: { tiers: [{ rate: 120 }], reject: 11 } }] },
      { A: 10, G: 85 },
    );
    const ash = merged.terms?.clauses.find((clause) => clause.indicator === "A");
    expect(ash?.penalty.tiers[0].rate).toBe(120);
    expect(ash?.penalty.reject).toBe(11);
    const cohesion = merged.terms?.clauses.find((clause) => clause.indicator === "G");
    expect(cohesion?.penalty.tiers[0].rate).toBe(5);
    expect(merged.terms?.contract_moisture).toBe(8);
  });

  it("只给部分指标保证值时, 只产出对应条款, 未提供保证值的指标既不出条款也不算孤儿", () => {
    const merged = mergePurchaseTerms(template, undefined, { A: 10 });
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.terms?.clauses[0].indicator).toBe("A");
    expect(merged.terms?.clauses.some((clause) => clause.indicator === "G")).toBe(false);
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("单煤覆盖可以新增模板没有的指标(覆盖专属条款)", () => {
    const merged = mergePurchaseTerms(
      template,
      { clauses: [{ indicator: "M", direction: "Upper", penalty: { tiers: [{ rate: 30 }], reject: 15 } }] },
      { M: 9 },
    );
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.terms?.clauses[0].indicator).toBe("M");
    expect(merged.terms?.clauses[0].penalty.tiers[0].rate).toBe(30);
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("单煤覆盖可以排除模板条款: 即使有保证值也不产出, 且不计入孤儿", () => {
    const merged = mergePurchaseTerms(
      template,
      { excluded_indicators: ["G"] },
      { A: 10, G: 85 },
    );
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.terms?.clauses.some((clause) => clause.indicator === "G")).toBe(false);
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("模板整体缺失(如跨设备未同步): 有保证值却找不到条款要报孤儿, 不能静默吃掉", () => {
    const merged = mergePurchaseTerms(null, undefined, { A: 10 });
    expect(merged.terms).toBeNull();
    expect(merged.orphanedGuarantees).toEqual(["A"]);
  });

  it("部分指标的模板条款缺失: 有条款的照常产出, 缺条款的报孤儿而不是被吞掉", () => {
    // 模拟第二台设备: 全局模板没同步过来(null), 但该煤自己的覆盖条款
    // 随 coal_prefs 正常同步到了(补上 A), G 只能指望模板, 模板没了.
    const merged = mergePurchaseTerms(
      null,
      { clauses: [{ indicator: "A", direction: "Upper", penalty: { tiers: [{ rate: 80 }], reject: 12 } }] },
      { A: 10, G: 85 },
    );
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.terms?.clauses[0].indicator).toBe("A");
    expect(merged.orphanedGuarantees).toEqual(["G"]);
  });

  it("覆盖显式把合同水分设为 null 时关闭该项, 不回退模板", () => {
    const merged = mergePurchaseTerms(template, { contract_moisture: null }, { A: 10 });
    expect(merged.terms?.contract_moisture).toBeNull();
  });

  it("覆盖完全不提合同水分这个键时, 回退模板的值", () => {
    const merged = mergePurchaseTerms(template, { clauses: [] }, { A: 10 });
    expect(merged.terms?.contract_moisture).toBe(8);
  });
});
