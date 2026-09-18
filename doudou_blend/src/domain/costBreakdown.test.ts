import { describe, expect, it } from "vitest";
import type { CostBreakdown } from "../types";
import { hasCostAdjustments } from "./costBreakdown";

const base: CostBreakdown = {
  fob_per_ton: 1000,
  frt_per_ton: 100,
  cif_per_ton: 1100,
  purchase_adjust_per_ton: 0,
  penalty_per_ton: 0,
  net_per_ton: 1100,
};

describe("hasCostAdjustments", () => {
  it("买入修正与卖出扣款都是 0 时返回 false", () => {
    expect(hasCostAdjustments(base)).toBe(false);
  });

  it("有买入修正时返回 true", () => {
    expect(hasCostAdjustments({ ...base, purchase_adjust_per_ton: -40 })).toBe(
      true,
    );
  });

  it("有卖出扣款时返回 true", () => {
    expect(hasCostAdjustments({ ...base, penalty_per_ton: 64 })).toBe(true);
  });

  it("字段缺失 (老记录) 时按 0 处理, 返回 false", () => {
    const legacy: CostBreakdown = {
      fob_per_ton: 900,
      frt_per_ton: 100,
      cif_per_ton: 1000,
    };
    expect(hasCostAdjustments(legacy)).toBe(false);
  });

  it("买入修正与卖出扣款刚好互相抵消 (net===cif) 时仍返回 true —— 抵消不等于没发生", () => {
    // 这是本函数存在的理由: |net-cif| 这种"事后看合计"的判定会漏掉这种情况,
    // 因为两笔调整数值上抵消了, 但用户确实各发生了一笔买入折扣和一笔卖出扣款,
    // 理应都展示出来, 而不是被"合计没差异"糊弄成什么都没发生过。
    const cancelledOut: CostBreakdown = {
      ...base,
      purchase_adjust_per_ton: -30,
      penalty_per_ton: 30,
      net_per_ton: 1100, // === cif_per_ton
    };
    expect(hasCostAdjustments(cancelledOut)).toBe(true);
  });
});
