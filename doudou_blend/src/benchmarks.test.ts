import { describe, expect, it } from "vitest";
import { solverPriceFactor } from "./benchmarks";
import type { MasterPriceFactor } from "./types";

describe("solverPriceFactor", () => {
  it("去掉只给人看、只给服务端用的字段, 系数表与基准原样保留", () => {
    const master: MasterPriceFactor = {
      adjusts: [],
      benchmarks: [{ name: "晋中", props: { S: 1.3 }, price: 2050, date: "2026-10-15" }],
      description: "说明",
      uncertainty_note: "说明",
      benchmarks_note: "说明",
      index_series_note: "说明",
      index_series: [
        { code: "LJM_FM", type_name: "肥煤", name: "钢铁网肥煤指数", coal_type: "肥煤", props: { S: 1.8 } },
      ],
    };
    const factor = solverPriceFactor(master);
    expect(Object.keys(factor).sort()).toEqual(["adjusts", "benchmarks"]);
    expect(factor.benchmarks?.[0].name).toBe("晋中");
  });
});
