import { describe, expect, it } from "vitest";
import { changeOver, lastYear, specText, staleDays } from "./mcci";

const points = [
  { date: "2026-10-16", price: 2000 },
  { date: "2026-10-19", price: 2010 },
  { date: "2026-10-23", price: 2020 },
  { date: "2026-10-26", price: 2030 },
];

describe("mcci 指数换算", () => {
  it("日涨跌比上一个交易日, 周涨跌比 7 天前 (含) 最近的一天", () => {
    expect(changeOver(points, 0)?.toFixed(4)).toBe((((2030 - 2020) / 2020) * 100).toFixed(4));
    // 10/26 往前 7 天是 10/19.
    expect(changeOver(points, 7)?.toFixed(4)).toBe((((2030 - 2010) / 2010) * 100).toFixed(4));
    expect(changeOver(points.slice(0, 1), 0)).toBeNull();
    // 中间断了数据: 7 天前往前一周内都没有点, 就不硬拿几周前的比.
    const gap = [{ date: "2026-09-20", price: 1800 }, { date: "2026-10-26", price: 2030 }];
    expect(changeOver(gap, 7)).toBeNull();
    expect(changeOver([], 7)).toBeNull();
  });

  it("停更天数按日历天算", () => {
    expect(staleDays("2026-10-26", new Date("2026-10-27T20:00:00+08:00"))).toBe(1);
    expect(staleDays("2026-10-20", new Date("2026-10-27T09:00:00+08:00"))).toBe(7);
  });

  it("只留近一年", () => {
    const old = [{ date: "2025-10-01", price: 1 }, ...points];
    expect(lastYear(old).map((p) => p.date)[0]).toBe("2026-10-16");
  });

  it("标准品指标按 A V S G Mt 排", () => {
    expect(specText({ S: 1.8, A: 10, V: 30, G: 90, M: 10 })).toBe("A10 V30 S1.8 G90 Mt10");
  });
});
