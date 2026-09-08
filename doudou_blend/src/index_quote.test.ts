import { describe, expect, it } from "vitest";
import { parseEastmoneyKlines, parseRealtimeQuote } from "./index_quote";

/** 2026-09-08 焦煤主连实测响应: 1664.0, 涨 19.5, +1.19% (昨结算 1644.5) */
const jmmResponse = {
  rc: 0,
  data: { f43: 16640, f58: "焦煤主连", f59: 1, f169: 195, f170: 119 },
};

describe("parseRealtimeQuote", () => {
  it("按 f59 位小数还原价格与涨跌额", () => {
    expect(parseRealtimeQuote(jmmResponse)).toEqual({
      price: 1664,
      change: 19.5,
      pct: 1.19,
    });
  });

  it("涨跌幅固定除以 100, 不受 f59 影响", () => {
    // 螺纹钢 decimal=0, 3170 元整; 涨跌幅仍是 f170/100
    const rb = { data: { f43: 3170, f59: 0, f169: 12, f170: 38 } };
    expect(parseRealtimeQuote(rb)).toEqual({
      price: 3170,
      change: 12,
      pct: 0.38,
    });
  });

  it("涨跌幅取的是昨结算口径, 与相邻收盘差不同", () => {
    // 昨收盘 1627.5 -> 现价 1664.0 是 +2.24%, 但交易所口径(昨结算 1644.5)是 +1.19%
    const quote = parseRealtimeQuote(jmmResponse);
    expect(quote?.pct).toBe(1.19);
    expect(quote?.price! - quote?.change!).toBeCloseTo(1644.5, 6);
  });

  it("f59 缺失时按 0 位小数处理", () => {
    const noDecimal = { data: { f43: 1500, f169: 10, f170: 67 } };
    expect(parseRealtimeQuote(noDecimal)).toMatchObject({ price: 1500 });
  });

  it("停牌/无成交回 0 价时不当作真实行情", () => {
    expect(
      parseRealtimeQuote({ data: { f43: 0, f59: 1, f169: 0, f170: 0 } }),
    ).toBeNull();
  });

  it("缺必需字段返回 null", () => {
    expect(parseRealtimeQuote({ data: { f43: 16640, f59: 1 } })).toBeNull();
  });

  it("data 为 null / 非对象 / 空响应都返回 null", () => {
    expect(parseRealtimeQuote({ data: null })).toBeNull();
    expect(parseRealtimeQuote({ data: [] })).toBeNull();
    expect(parseRealtimeQuote({})).toBeNull();
    expect(parseRealtimeQuote(null)).toBeNull();
  });
});

describe("parseEastmoneyKlines", () => {
  const response = {
    data: {
      klines: [
        "2026-09-04,1658.0,1666.0,1694.5,1647.0,1025294,102785114112.0",
        "2026-09-07,1666.0,1627.5,1686.5,1608.0,1110524,109608308736.0",
        "2026-09-08,1622.0,1662.0,1664.0,1613.5,506070,49657788480.0",
      ],
    },
  };

  it("取第 2/3 个字段作为开盘价与收盘价", () => {
    expect(parseEastmoneyKlines(response, 3)).toEqual([
      { date: "2026-09-04", open: 1658, close: 1666 },
      { date: "2026-09-07", open: 1666, close: 1627.5 },
      { date: "2026-09-08", open: 1622, close: 1662 },
    ]);
  });

  it("只取末尾 take 条", () => {
    expect(parseEastmoneyKlines(response, 1)).toEqual([
      { date: "2026-09-08", open: 1622, close: 1662 },
    ]);
  });

  it("跳过脏行", () => {
    const dirty = { data: { klines: ["坏数据", 42, "2026-09-08,1600,1662,1,1"] } };
    expect(parseEastmoneyKlines(dirty, 10)).toEqual([
      { date: "2026-09-08", open: 1600, close: 1662 },
    ]);
  });

  it("结构不对时返回空数组而不是抛错", () => {
    expect(parseEastmoneyKlines({ data: { klines: null } }, 10)).toEqual([]);
    expect(parseEastmoneyKlines(null, 10)).toEqual([]);
  });
});
