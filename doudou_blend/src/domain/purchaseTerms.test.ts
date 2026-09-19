import { describe, expect, it } from "vitest";
import type { CoalPrefs } from "../storage";
import type { MasterCoalEntry } from "../types";
import type { PenaltyTemplate } from "../penalty";
import { resolveCoalPool } from "./resolvedCoal";
import {
  extractOrphanedGuarantees,
  purchaseTermsByCoalName,
  resolvePurchaseTerms,
} from "./purchaseTerms";

const masterCoal: MasterCoalEntry = {
  name: "测试主煤",
  region: "山西",
  coal_type: "焦煤",
  status: "verified",
  props: { S: 0.5, A: 10, G: 80 },
  fob: 1_000,
  frt: 100,
};

const secondCoal: MasterCoalEntry = {
  ...masterCoal,
  name: "第二煤",
  fob: 800,
};

const template: PenaltyTemplate = {
  clauses: [
    {
      indicator: "S",
      direction: "Upper",
      penalty: { tiers: [{ rate: 80 }], reject: 1.5 },
    },
  ],
};

describe("resolvePurchaseTerms (单次扫描, 同时产出请求条款与孤儿告警)", () => {
  it("模板能匹配到条款时, 该煤的 terms 非空且没有孤儿指标", () => {
    const prefs: CoalPrefs = {
      测试主煤: { purchase_guarantees: { S: 0.5 } },
    };
    const pool = resolveCoalPool([masterCoal], [], prefs);

    const entries = resolvePurchaseTerms(pool, prefs, template);

    expect(entries).toEqual([
      {
        coal: "测试主煤",
        terms: {
          clauses: [
            {
              indicator: "S",
              direction: "Upper",
              guarantee: 0.5,
              penalty: template.clauses[0].penalty,
            },
          ],
          contract_moisture: null,
          moisture_excess_double_threshold: null,
        },
        orphanedIndicators: [],
      },
    ]);
  });

  it("有保证值但模板/覆盖里找不到条款时, terms 为 null 且指标计入孤儿", () => {
    const prefs: CoalPrefs = {
      测试主煤: { purchase_guarantees: { S: 0.5 } },
    };
    const pool = resolveCoalPool([masterCoal], [], prefs);

    const entries = resolvePurchaseTerms(pool, prefs, null);

    expect(entries).toEqual([
      { coal: "测试主煤", terms: null, orphanedIndicators: ["S"] },
    ]);
  });

  it("没有保证值的煤 terms 为 null, 不计入孤儿 (没配置不是配置丢了)", () => {
    const pool = resolveCoalPool([masterCoal], [], {});

    const entries = resolvePurchaseTerms(pool, {}, null);

    expect(entries).toEqual([
      { coal: "测试主煤", terms: null, orphanedIndicators: [] },
    ]);
  });

  it("非 ready 的煤 (停用/隐藏/缺价) 不出现在结果里, 即使配了保证值", () => {
    const prefs: CoalPrefs = {
      测试主煤: { enabled: false, purchase_guarantees: { S: 0.5 } },
    };
    const pool = resolveCoalPool([masterCoal], [], prefs);

    expect(resolvePurchaseTerms(pool, prefs, null)).toEqual([]);
  });

  it("单煤显式排除的指标不计入孤儿, 该煤也没有其他条款时 terms 为 null", () => {
    const prefs: CoalPrefs = {
      测试主煤: {
        purchase_guarantees: { S: 0.5 },
        purchase_override: { excluded_indicators: ["S"] },
      },
    };
    const pool = resolveCoalPool([masterCoal], [], prefs);

    expect(resolvePurchaseTerms(pool, prefs, null)).toEqual([
      { coal: "测试主煤", terms: null, orphanedIndicators: [] },
    ]);
  });

  it("多种煤各自算各自的, 互不影响", () => {
    const prefs: CoalPrefs = {
      测试主煤: { purchase_guarantees: { S: 0.5 } },
      第二煤: { purchase_guarantees: { S: 0.5 } },
    };
    const pool = resolveCoalPool([masterCoal, secondCoal], [], prefs);

    const entries = resolvePurchaseTerms(pool, prefs, template);

    expect(entries.map((e) => e.coal).sort()).toEqual(["测试主煤", "第二煤"]);
    expect(entries.every((e) => e.terms != null && e.orphanedIndicators.length === 0)).toBe(true);
  });
});

describe("extractOrphanedGuarantees (从单次扫描派生告警)", () => {
  it("只保留有孤儿指标的条目, 转成 CostCard 要的 {coal, indicators} 形状", () => {
    const entries = resolvePurchaseTerms(
      resolveCoalPool(
        [masterCoal, secondCoal],
        [],
        {
          测试主煤: { purchase_guarantees: { S: 0.5, A: 10 } },
          第二煤: { purchase_guarantees: { G: 70 } },
        },
      ),
      {
        测试主煤: { purchase_guarantees: { S: 0.5, A: 10 } },
        第二煤: { purchase_guarantees: { G: 70 } },
      },
      null,
    );

    expect(extractOrphanedGuarantees(entries)).toEqual([
      { coal: "测试主煤", indicators: ["S", "A"] },
      { coal: "第二煤", indicators: ["G"] },
    ]);
  });

  it("没有孤儿时返回空数组", () => {
    const prefs: CoalPrefs = { 测试主煤: { purchase_guarantees: { S: 0.5 } } };
    const entries = resolvePurchaseTerms(
      resolveCoalPool([masterCoal], [], prefs),
      prefs,
      template,
    );

    expect(extractOrphanedGuarantees(entries)).toEqual([]);
  });
});

describe("purchaseTermsByCoalName (从单次扫描派生请求用的查找表)", () => {
  it("按煤名查得到 terms, 找不到条款的煤查到 null", () => {
    const prefs: CoalPrefs = {
      测试主煤: { purchase_guarantees: { S: 0.5 } },
      第二煤: { purchase_guarantees: { S: 0.5 } },
    };
    const pool = resolveCoalPool([masterCoal, secondCoal], [], prefs);
    // 只给"测试主煤"一个能匹配的模板, "第二煤"用同一个模板也能匹配 —— 换成
    // 只对"测试主煤"生效的场景: 用排除条款让"第二煤"变成孤儿(terms=null)。
    const overriddenPrefs: CoalPrefs = {
      ...prefs,
      第二煤: {
        purchase_guarantees: { S: 0.5 },
        purchase_override: { excluded_indicators: ["S"] },
      },
    };
    const entries = resolvePurchaseTerms(pool, overriddenPrefs, template);
    const map = purchaseTermsByCoalName(entries);

    expect(map.get("测试主煤")).not.toBeNull();
    expect(map.get("第二煤")).toBeNull();
    expect(map.get("不存在的煤")).toBeUndefined();
  });
});
