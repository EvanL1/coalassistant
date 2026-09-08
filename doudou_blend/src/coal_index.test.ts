import { describe, expect, it } from "vitest";
import {
  indexPriceOn,
  indexRatioSince,
  parseIndexSeries,
} from "./coal_index";

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

describe("indexPriceOn", () => {
  const points = parseIndexSeries(raw);

  it("命中当日", () => {
    expect(indexPriceOn(points, "2026-08-27")).toBe(1645);
  });

  it("无当日时取之前最近一点", () => {
    expect(indexPriceOn(points, "2026-06-30")).toBe(1503);
  });

  it("日期早于全部数据返回 null", () => {
    expect(indexPriceOn(points, "2020-01-01")).toBeNull();
  });
});

describe("indexRatioSince", () => {
  const points = parseIndexSeries(raw);

  it("现货无换月, 直接最新 ÷ 基准", () => {
    // 6-30 之前最近是 6-25 的 1503, 最新 9-03 的 1831
    expect(indexRatioSince(points, "2026-06-30")).toBeCloseTo(1831 / 1503, 6);
  });

  it("基准晚于全部数据时取最新点, 比例为 1", () => {
    expect(indexRatioSince(points, "2026-12-01")).toBe(1);
  });

  it("基准早于全部数据无法定位, 返回 null", () => {
    expect(indexRatioSince(points, "2020-01-01")).toBeNull();
  });

  it("空序列返回 null", () => {
    expect(indexRatioSince([], "2026-06-30")).toBeNull();
  });
});
