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

  it("同一指标既在覆盖条款里又被排除时, 排除生效(排除是比覆盖条款更具体的信号: 覆盖条款说明'这项按这个价算', 排除说明'这项压根不适用', 后者更接近用户的真实意图)", () => {
    const merged = mergePurchaseTerms(
      null,
      {
        clauses: [{ indicator: "A", direction: "Upper", penalty: { tiers: [{ rate: 120 }], reject: 11 } }],
        excluded_indicators: ["A"],
      },
      { A: 10 },
    );
    expect(merged.terms).toBeNull();
    expect(merged.orphanedGuarantees).toEqual([]);
  });
});

// contract_moisture / moisture_excess_double_threshold 共用同一条继承规则,
// 两个字段各测一遍三态 + JSON 往返稳定性 —— 只测 contract_moisture 曾经让
// moisture_excess_double_threshold 那条分支(删掉照样全绿)偷偷漏测过一次.
describe.each([
  ["contract_moisture", 8] as const,
  ["moisture_excess_double_threshold", 12] as const,
])("mergePurchaseTerms — %s 的三态继承", (field, templateValue) => {
  it("键缺失时继承模板", () => {
    const merged = mergePurchaseTerms(template, { clauses: [] }, { A: 10 });
    expect(merged.terms?.[field]).toBe(templateValue);
  });

  it("键存在但值为 undefined 时视同缺失, 继承模板", () => {
    const merged = mergePurchaseTerms(template, { clauses: [], [field]: undefined }, { A: 10 });
    expect(merged.terms?.[field]).toBe(templateValue);
  });

  it("键存在且值为 null 时显式关闭, 不回退模板", () => {
    const merged = mergePurchaseTerms(template, { [field]: null }, { A: 10 });
    expect(merged.terms?.[field]).toBeNull();
  });

  it("JSON 往返后行为不变(JSON.stringify 会丢掉值为 undefined 的键, 往返前后必须算出同一个结果)", () => {
    const override = { clauses: [], [field]: undefined };
    const before = mergePurchaseTerms(template, override, { A: 10 });
    const roundTripped = JSON.parse(JSON.stringify(override));
    const after = mergePurchaseTerms(template, roundTripped, { A: 10 });
    expect(after.terms?.[field]).toBe(before.terms?.[field]);
    expect(before.terms?.[field]).toBe(templateValue); // 双重确认: 往返前后都是"继承", 不是巧合地都错
  });
});
