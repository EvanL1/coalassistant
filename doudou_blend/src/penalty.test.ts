import { describe, expect, it } from "vitest";
import {
  deviationNoun,
  guaranteeIssue,
  indicatorUnit,
  mergePurchaseTerms,
  penaltyFromDraft,
  penaltyToDraft,
  templateFromDraft,
  tierRate,
} from "./penalty";
import type { PenaltyDraft, PenaltyTemplate } from "./penalty";

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

describe("indicatorUnit", () => {
  it("粘结与焦炭强度按点计, 其余按百分点计", () => {
    expect(indicatorUnit("G")).toBe("点");
    expect(indicatorUnit("CSR")).toBe("点");
    expect(indicatorUnit("A")).toBe("%");
    expect(indicatorUnit("S")).toBe("%");
  });
});

/** 合同原文: 灰分 ≤10%, 每超 0.1% 扣 8 元/吨, 超过 12% 拒收. */
const ashDraft: PenaltyDraft = {
  tiers: [{ step: "0.1", amount: "8", width: "" }],
  reject: "12",
};

const ashOptions = {
  indicator: "A",
  direction: "Upper" as const,
  bound: 10,
  boundLabel: "合同上限",
};

describe("penaltyFromDraft 单位换算", () => {
  it("照抄合同的每 0.1% 扣 8 元, 换算成 rate 80", () => {
    const { penalty, error } = penaltyFromDraft(ashDraft, ashOptions);
    expect(error).toBeNull();
    expect(penalty).toEqual({ tiers: [{ rate: 80 }], reject: 12 });
  });

  it("末档不带 width —— core 用缺席表示无上限", () => {
    const { penalty } = penaltyFromDraft(ashDraft, ashOptions);
    expect(penalty?.tiers[0]).not.toHaveProperty("width");
  });

  it("多档: 每档各自换算, 非末档带 width", () => {
    const draft: PenaltyDraft = {
      tiers: [
        { step: "0.1", amount: "8", width: "0.5" },
        { step: "0.1", amount: "15", width: "" },
      ],
      reject: "12",
    };
    const { penalty, error } = penaltyFromDraft(draft, ashOptions);
    expect(error).toBeNull();
    expect(penalty).toEqual({
      tiers: [{ rate: 80, width: 0.5 }, { rate: 150 }],
      reject: 12,
    });
  });

  it("扣款金额留空时报错, 不能当成 0 元/吨悄悄存下去", () => {
    const draft: PenaltyDraft = {
      tiers: [{ step: "0.1", amount: "", width: "" }],
      reject: "12",
    };
    const { penalty, error } = penaltyFromDraft(draft, ashOptions);
    expect(penalty).toBeNull();
    expect(error).toContain("扣");
  });

  it("扣款金额为负时报错 —— 负扣款等于倒贴钱", () => {
    const draft: PenaltyDraft = {
      tiers: [{ step: "0.1", amount: "-8", width: "" }],
      reject: "12",
    };
    expect(penaltyFromDraft(draft, ashOptions).penalty).toBeNull();
  });

  it("0 元/吨是合法的零扣款档位, 不是错误", () => {
    const draft: PenaltyDraft = {
      tiers: [
        { step: "0.1", amount: "0", width: "0.5" },
        { step: "0.1", amount: "8", width: "" },
      ],
      reject: "12",
    };
    const { penalty, error } = penaltyFromDraft(draft, ashOptions);
    expect(error).toBeNull();
    expect(penalty?.tiers[0].rate).toBe(0);
  });

  it("「每」栏留空或填 0 时报错, 不能除出 Infinity", () => {
    for (const step of ["", "0", "-0.1"]) {
      const draft: PenaltyDraft = {
        tiers: [{ step, amount: "8", width: "" }],
        reject: "12",
      };
      const { penalty, error } = penaltyFromDraft(draft, ashOptions);
      expect(penalty).toBeNull();
      expect(error).toContain("每");
    }
  });
});

describe("penaltyFromDraft 与 Rust validate_penalty 同规则", () => {
  it("一档都没有时报错", () => {
    const { penalty, error } = penaltyFromDraft(
      { tiers: [], reject: "12" },
      ashOptions,
    );
    expect(penalty).toBeNull();
    expect(error).toContain("一档");
  });

  it("后一档扣款率不高于前一档时报错(凸性: 档位必须一档比一档狠)", () => {
    const draft: PenaltyDraft = {
      tiers: [
        { step: "0.1", amount: "8", width: "0.5" },
        { step: "0.1", amount: "6", width: "" },
      ],
      reject: "12",
    };
    const { penalty, error } = penaltyFromDraft(draft, ashOptions);
    expect(penalty).toBeNull();
    expect(error).toContain("第 2 档");
  });

  it("换算后扣款率相等也不行 —— Rust 要求严格递增", () => {
    const draft: PenaltyDraft = {
      tiers: [
        { step: "0.1", amount: "8", width: "0.5" },
        { step: "1", amount: "80", width: "" },
      ],
      reject: "12",
    };
    expect(penaltyFromDraft(draft, ashOptions).penalty).toBeNull();
  });

  it("非末档缺「本档覆盖」宽度时报错", () => {
    const draft: PenaltyDraft = {
      tiers: [
        { step: "0.1", amount: "8", width: "" },
        { step: "0.1", amount: "15", width: "" },
      ],
      reject: "12",
    };
    const { penalty, error } = penaltyFromDraft(draft, ashOptions);
    expect(penalty).toBeNull();
    expect(error).toContain("第 1 档");
  });

  it("非末档宽度必须为正数", () => {
    const draft: PenaltyDraft = {
      tiers: [
        { step: "0.1", amount: "8", width: "0" },
        { step: "0.1", amount: "15", width: "" },
      ],
      reject: "12",
    };
    expect(penaltyFromDraft(draft, ashOptions).penalty).toBeNull();
  });

  it("末档填了宽度时报错 —— 末档吃掉剩余全部超出", () => {
    const draft: PenaltyDraft = {
      tiers: [{ step: "0.1", amount: "8", width: "0.5" }],
      reject: "12",
    };
    const { penalty, error } = penaltyFromDraft(draft, ashOptions);
    expect(penalty).toBeNull();
    expect(error).toContain("末档");
  });

  it("拒收线留空时报错", () => {
    const { penalty, error } = penaltyFromDraft(
      { ...ashDraft, reject: "" },
      ashOptions,
    );
    expect(penalty).toBeNull();
    expect(error).toContain("拒收线");
  });

  it("上限型: 拒收线低于合同上限时报错, 等于上限时通过", () => {
    expect(
      penaltyFromDraft({ ...ashDraft, reject: "9" }, ashOptions).penalty,
    ).toBeNull();
    expect(
      penaltyFromDraft({ ...ashDraft, reject: "10" }, ashOptions).error,
    ).toBeNull();
  });

  it("下限型: 拒收线高于合同下限时报错, 等于下限时通过", () => {
    const options = {
      indicator: "G",
      direction: "Lower" as const,
      bound: 75,
      boundLabel: "合同下限",
    };
    const draft: PenaltyDraft = {
      tiers: [{ step: "1", amount: "5", width: "" }],
      reject: "80",
    };
    expect(penaltyFromDraft(draft, options).penalty).toBeNull();
    expect(penaltyFromDraft({ ...draft, reject: "75" }, options).error).toBeNull();
  });

  it("区间型指标不支持计价", () => {
    const { penalty, error } = penaltyFromDraft(ashDraft, {
      ...ashOptions,
      direction: "Range",
    });
    expect(penalty).toBeNull();
    expect(error).toContain("区间");
  });

  it("边界未知(采购模板: 保证值逐煤不同)时跳过拒收线比对, 其余规则照查", () => {
    const options = { ...ashOptions, bound: null, boundLabel: "保证值" };
    expect(penaltyFromDraft({ ...ashDraft, reject: "1" }, options).error).toBeNull();
    expect(
      penaltyFromDraft({ tiers: [], reject: "1" }, options).penalty,
    ).toBeNull();
  });
});

describe("penaltyToDraft", () => {
  it("回显成每 1 单位扣 rate 元 —— 原文的每 0.1% 无法还原, 但换算等价且不引入浮点噪声", () => {
    expect(penaltyToDraft({ tiers: [{ rate: 80 }], reject: 12 })).toEqual({
      tiers: [{ step: "1", amount: "80", width: "" }],
      reject: "12",
    });
  });

  it("非末档的宽度照样回显", () => {
    const draft = penaltyToDraft({
      tiers: [{ rate: 80, width: 0.5 }, { rate: 150 }],
      reject: 12,
    });
    expect(draft.tiers).toEqual([
      { step: "1", amount: "80", width: "0.5" },
      { step: "1", amount: "150", width: "" },
    ]);
  });

  it("往返不改变 core 看到的条款", () => {
    const penalty = { tiers: [{ rate: 80, width: 0.5 }, { rate: 150 }], reject: 12 };
    const back = penaltyFromDraft(penaltyToDraft(penalty), ashOptions);
    expect(back.penalty).toEqual(penalty);
  });
});

describe("guaranteeIssue", () => {
  const upperOnly: PenaltyTemplate = {
    clauses: [
      {
        indicator: "A",
        direction: "Upper",
        penalty: { tiers: [{ rate: 80 }], reject: 12 },
      },
    ],
  };

  it("保证值在拒收线之内时没有问题", () => {
    expect(
      guaranteeIssue({
        template: upperOnly,
        override: undefined,
        indicator: "A",
        guarantee: 10,
      }),
    ).toBeNull();
  });

  it("上限型: 保证值高于拒收线时报错 —— 这条条款 core 会直接拒绝", () => {
    const issue = guaranteeIssue({
      template: upperOnly,
      override: undefined,
      indicator: "A",
      guarantee: 13,
    });
    expect(issue?.level).toBe("error");
    expect(issue?.message).toContain("拒收线");
  });

  it("下限型: 保证值低于拒收线时报错", () => {
    const lower: PenaltyTemplate = {
      clauses: [
        {
          indicator: "G",
          direction: "Lower",
          penalty: { tiers: [{ rate: 5 }], reject: 80 },
        },
      ],
    };
    const issue = guaranteeIssue({
      template: lower,
      override: undefined,
      indicator: "G",
      guarantee: 75,
    });
    expect(issue?.level).toBe("error");
  });

  it("没有对应条款时只提示不拦 —— 用户可能先填保证值再配模板", () => {
    const issue = guaranteeIssue({
      template: null,
      override: undefined,
      indicator: "A",
      guarantee: 10,
    });
    expect(issue?.level).toBe("hint");
    expect(issue?.message).toContain("采购扣款模板");
  });

  it("被单煤排除的指标不提示 —— 那是用户主动排除, 不是漏配", () => {
    expect(
      guaranteeIssue({
        template: upperOnly,
        override: { excluded_indicators: ["A"] },
        indicator: "A",
        guarantee: 10,
      }),
    ).toBeNull();
  });

  it("水分已按扣量计时不能再填水分保证值 —— core 明确禁止两种机制并用", () => {
    const both: PenaltyTemplate = {
      contract_moisture: 8,
      clauses: [
        {
          indicator: "M",
          direction: "Upper",
          penalty: { tiers: [{ rate: 30 }], reject: 14 },
        },
      ],
    };
    const issue = guaranteeIssue({
      template: both,
      override: undefined,
      indicator: "M",
      guarantee: 9,
    });
    expect(issue?.level).toBe("error");
    expect(issue?.message).toContain("水分");
  });

  it("保证值不是数时报错", () => {
    const issue = guaranteeIssue({
      template: upperOnly,
      override: undefined,
      indicator: "A",
      guarantee: Number.NaN,
    });
    expect(issue?.level).toBe("error");
  });
});

describe("templateFromDraft", () => {
  const ashClause = {
    indicator: "A",
    direction: "Upper" as const,
    penalty: { tiers: [{ step: "0.1", amount: "8", width: "" }], reject: "12" },
  };

  it("照抄的每 0.1% 扣 8 元, 模板里存成 80 元/吨·%", () => {
    const { template, error } = templateFromDraft({
      clauses: [ashClause],
      contractMoisture: "",
      doubleThreshold: "",
    });
    expect(error).toBeNull();
    expect(template?.clauses[0].penalty).toEqual({ tiers: [{ rate: 80 }], reject: 12 });
  });

  it("合同水分与双倍阈值照常带进模板, 留空则不设", () => {
    const filled = templateFromDraft({
      clauses: [ashClause],
      contractMoisture: "8",
      doubleThreshold: "12",
    });
    expect(filled.template?.contract_moisture).toBe(8);
    expect(filled.template?.moisture_excess_double_threshold).toBe(12);

    const blank = templateFromDraft({
      clauses: [ashClause],
      contractMoisture: "",
      doubleThreshold: "",
    });
    expect(blank.template?.contract_moisture).toBeNull();
    expect(blank.template?.moisture_excess_double_threshold).toBeNull();
  });

  it("同一指标两条条款时报错 —— core 会把两条扣款各算一遍, 把煤价算便宜", () => {
    const { template, error } = templateFromDraft({
      clauses: [ashClause, { ...ashClause }],
      contractMoisture: "",
      doubleThreshold: "",
    });
    expect(template).toBeNull();
    expect(error).toContain("灰");
  });

  it("某条条款的档位填错时, 报错要指名道姓是哪一项", () => {
    const { template, error } = templateFromDraft({
      clauses: [
        ashClause,
        {
          indicator: "S",
          direction: "Upper",
          penalty: { tiers: [{ step: "0.01", amount: "", width: "" }], reject: "1" },
        },
      ],
      contractMoisture: "",
      doubleThreshold: "",
    });
    expect(template).toBeNull();
    expect(error).toContain("硫");
  });

  it("合同水分不在 0~100 之间时报错", () => {
    const { template, error } = templateFromDraft({
      clauses: [ashClause],
      contractMoisture: "120",
      doubleThreshold: "",
    });
    expect(template).toBeNull();
    expect(error).toContain("合同水分");
  });

  it("双倍阈值不在 0~100 之间时报错", () => {
    expect(
      templateFromDraft({
        clauses: [ashClause],
        contractMoisture: "8",
        doubleThreshold: "-1",
      }).template,
    ).toBeNull();
  });

  it("一条条款都没有时报错 —— 只填水分不会影响任何成本", () => {
    const { template, error } = templateFromDraft({
      clauses: [],
      contractMoisture: "8",
      doubleThreshold: "",
    });
    expect(template).toBeNull();
    expect(error).toContain("清空模板");
  });

  it("模板里的拒收线不跟任何边界比 —— 保证值逐煤不同, 那一步留给煤卡录保证值时查", () => {
    const { error } = templateFromDraft({
      clauses: [{ ...ashClause, penalty: { ...ashClause.penalty, reject: "0.5" } }],
      contractMoisture: "",
      doubleThreshold: "",
    });
    expect(error).toBeNull();
  });
});

describe("下限型指标的提示用词", () => {
  const lowerOptions = {
    indicator: "G",
    direction: "Lower" as const,
    bound: 75,
    boundLabel: "合同下限",
  };

  it("下限型说的是「不足」而不是「超出」—— 粘结差 5 点是不够, 不是超标", () => {
    const { error } = penaltyFromDraft(
      {
        tiers: [
          { step: "1", amount: "5", width: "" },
          { step: "1", amount: "9", width: "" },
        ],
        reject: "70",
      },
      lowerOptions,
    );
    expect(error).toContain("不足");
    expect(error).not.toContain("超出");
  });

  it("上限型仍然说「超出」", () => {
    const { error } = penaltyFromDraft(
      {
        tiers: [
          { step: "0.1", amount: "8", width: "" },
          { step: "0.1", amount: "15", width: "" },
        ],
        reject: "12",
      },
      { indicator: "A", direction: "Upper", bound: 10, boundLabel: "合同上限" },
    );
    expect(error).toContain("超出");
    expect(error).not.toContain("不足");
  });

  it("deviationNoun 一处定义, 界面文案与报错文案共用", () => {
    expect(deviationNoun("Upper")).toBe("超出");
    expect(deviationNoun("Lower")).toBe("不足");
  });
});
