import { describe, expect, it } from "vitest";
import {
  anchorPriceOn,
  buildPriceAnchor,
  driftRatio,
  driftedFob,
  type PriceAnchor,
} from "./priceDrift";

/** 临北 6-30 报 1425, 9-8 复录 1650 -> 比例 1.1579 */
const singleAnchor: PriceAnchor = {
  coals: ["临北"],
  quotes: {
    临北: [
      { date: "2026-06-30", fob: 1425 },
      { date: "2026-09-08", fob: 1650 },
    ],
  },
};

/** 三个主力煤同时做锚点, 涨幅不同: 临北 +15.79%, 豹子沟 +20%, 大佛寺 +10% */
const multiAnchor: PriceAnchor = {
  coals: ["临北", "豹子沟", "大佛寺"],
  quotes: {
    临北: [
      { date: "2026-06-30", fob: 1425 },
      { date: "2026-09-08", fob: 1650 },
    ],
    豹子沟: [
      { date: "2026-06-30", fob: 1280 },
      { date: "2026-09-08", fob: 1536 },
    ],
    大佛寺: [
      { date: "2026-06-30", fob: 1150 },
      { date: "2026-09-08", fob: 1265 },
    ],
  },
};

describe("anchorPriceOn", () => {
  const quotes = [
    { date: "2026-06-30", fob: 1425 },
    { date: "2026-08-15", fob: 1500 },
    { date: "2026-09-08", fob: 1650 },
  ];

  it("命中当日报价", () => {
    expect(anchorPriceOn(quotes, "2026-08-15")).toBe(1500);
  });

  it("无当日报价时取之前最近一次", () => {
    expect(anchorPriceOn(quotes, "2026-08-20")).toBe(1500);
  });

  it("日期晚于所有报价时取最新一次", () => {
    expect(anchorPriceOn(quotes, "2026-12-01")).toBe(1650);
  });

  it("日期早于所有报价时无法定位, 返回 null", () => {
    expect(anchorPriceOn(quotes, "2026-01-01")).toBeNull();
  });

  it("报价乱序时仍按日期定位", () => {
    const shuffled = [quotes[2], quotes[0], quotes[1]];
    expect(anchorPriceOn(shuffled, "2026-08-20")).toBe(1500);
  });

  it("空报价返回 null", () => {
    expect(anchorPriceOn([], "2026-09-08")).toBeNull();
  });
});

describe("driftRatio", () => {
  it("单锚点按最新价与录价日价的比值", () => {
    expect(driftRatio(singleAnchor, "2026-06-30")).toBeCloseTo(1650 / 1425, 6);
  });

  it("多锚点取各自比例的平均", () => {
    const expected = (1650 / 1425 + 1536 / 1280 + 1265 / 1150) / 3;
    expect(driftRatio(multiAnchor, "2026-06-30")).toBeCloseTo(expected, 6);
  });

  it("录价日晚于所有锚点报价时不漂移", () => {
    expect(driftRatio(singleAnchor, "2026-12-01")).toBe(1);
  });

  it("锚点只有一条报价时不漂移", () => {
    const fresh: PriceAnchor = {
      coals: ["临北"],
      quotes: { 临北: [{ date: "2026-09-08", fob: 1650 }] },
    };
    expect(driftRatio(fresh, "2026-09-08")).toBe(1);
  });

  it("录价日早于锚点全部报价时无法推算, 返回 null", () => {
    expect(driftRatio(singleAnchor, "2026-01-01")).toBeNull();
  });

  it("没有锚点煤时返回 null", () => {
    expect(driftRatio({ coals: [], quotes: {} }, "2026-06-30")).toBeNull();
  });

  it("锚点基准价为 0 时跳过该锚点, 避免除零", () => {
    const broken: PriceAnchor = {
      coals: ["临北", "豹子沟"],
      quotes: {
        临北: [
          { date: "2026-06-30", fob: 0 },
          { date: "2026-09-08", fob: 1650 },
        ],
        豹子沟: multiAnchor.quotes.豹子沟,
      },
    };
    expect(driftRatio(broken, "2026-06-30")).toBeCloseTo(1536 / 1280, 6);
  });

  it("全部锚点都无效时返回 null", () => {
    const broken: PriceAnchor = {
      coals: ["临北"],
      quotes: { 临北: [{ date: "2026-06-30", fob: 0 }] },
    };
    expect(driftRatio(broken, "2026-06-30")).toBeNull();
  });
});

describe("buildPriceAnchor", () => {
  it("只收被标记为锚点的煤", () => {
    const anchor = buildPriceAnchor({
      临北: {
        is_price_anchor: true,
        fob_history: [{ date: "2026-06-30", fob: 1425 }],
      },
      古交浮精: {
        fob_history: [{ date: "2026-06-30", fob: 970 }],
      },
    });
    expect(anchor.coals).toEqual(["临北"]);
    expect(anchor.quotes.古交浮精).toBeUndefined();
  });

  it("锚点没有报价历史时不计入", () => {
    const anchor = buildPriceAnchor({
      临北: { is_price_anchor: true },
      豹子沟: { is_price_anchor: true, fob_history: [] },
    });
    expect(anchor.coals).toEqual([]);
  });

  it("过滤掉历史里的脏数据", () => {
    const anchor = buildPriceAnchor({
      临北: {
        is_price_anchor: true,
        fob_history: [
          { date: "2026-06-30", fob: 1425 },
          { date: "2026-07-15", fob: Number.NaN },
          null as never,
          { date: "2026-09-08", fob: 1650 },
        ],
      },
    });
    expect(anchor.quotes.临北).toEqual([
      { date: "2026-06-30", fob: 1425 },
      { date: "2026-09-08", fob: 1650 },
    ]);
  });

  it("空偏好返回空锚点, driftRatio 随之退化为不漂移", () => {
    const anchor = buildPriceAnchor({});
    expect(anchor).toEqual({ coals: [], quotes: {} });
    expect(driftRatio(anchor, "2026-06-30")).toBeNull();
  });
});

describe("driftedFob", () => {
  it("按锚点比例推算未更新煤的现价", () => {
    // 古交浮精 6-30 报 970, 锚点涨 15.79% -> 1123.2
    expect(driftedFob(970, "2026-06-30", singleAnchor)).toBeCloseTo(
      970 * (1650 / 1425),
      6,
    );
  });

  it("锚点煤自己不被漂移", () => {
    expect(driftedFob(1650, "2026-09-08", singleAnchor)).toBe(1650);
  });

  it("缺录价日期时不推算, 返回 null", () => {
    expect(driftedFob(970, null, singleAnchor)).toBeNull();
  });

  it("无法推算时返回 null 而不是瞎猜", () => {
    expect(driftedFob(970, "2026-01-01", singleAnchor)).toBeNull();
  });
});
