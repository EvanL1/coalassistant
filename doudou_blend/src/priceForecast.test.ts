import { describe, expect, it } from "vitest";
import { changeMark, isAhead } from "./priceForecast";

describe("预测价", () => {
  it("预测的那天要在今天 (北京时间) 之后才算数", () => {
    const now = new Date("2026-11-02T23:30:00+08:00");
    expect(isAhead("2026-11-03", now)).toBe(true);
    expect(isAhead("2026-11-02", now)).toBe(false);
    expect(isAhead("2026-10-30", now)).toBe(false);
    // 北京时间已是 11-03 (UTC 还在 11-02).
    expect(isAhead("2026-11-03", new Date("2026-11-03T00:10:00+08:00"))).toBe(false);
  });

  it.each([[12.3, "▲12"], [-8.6, "▼9"], [0.4, "持平"], [-0.4, "持平"], [0, "持平"]])(
    "涨跌按元取整 %s", (change, mark) => {
      expect(changeMark(change)).toBe(mark);
    });
});
