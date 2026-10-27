import { describe, expect, it } from "vitest";
import { parseIndexSeries } from "./coal_index";

/** CCP 现货指数实测片段 (乱序, 含脏数据) */
const raw = [
  { date: "2026-09-03", price: 1831 },
  { date: "2026-06-25", price: 1503 },
  { date: "2026-08-27", price: 1645 },
  { date: "2026-09-04", price: 0 }, // 无效
  { date: "2026-09-05", price: Number.NaN }, // 无效
  { price: 1700 }, // 缺 date
];

describe("parseIndexSeries", () => {
  it("过滤脏数据并按日期升序", () => {
    expect(parseIndexSeries(raw)).toEqual([
      { date: "2026-06-25", price: 1503 },
      { date: "2026-08-27", price: 1645 },
      { date: "2026-09-03", price: 1831 },
    ]);
  });

  it("非数组返回空", () => {
    expect(parseIndexSeries(null)).toEqual([]);
    expect(parseIndexSeries({})).toEqual([]);
  });
});
