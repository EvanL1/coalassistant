import { describe, expect, it } from "vitest";
import type { Spec } from "../types";
import { isContractIncomplete, withoutUnusedBound } from "./contract";

const spec = (over: Partial<{ direction: "Upper" | "Lower" | "Range"; min: number | null; max: number | null; enabled: boolean }>) => ({
  direction: "Range" as const,
  min: null,
  max: null,
  enabled: true,
  ...over,
});

describe("合同是否填完整 (与核心 quality.rs 同规则)", () => {
  it("上限型只看上限, 下限型只看下限", () => {
    expect(isContractIncomplete([spec({ direction: "Upper", max: 10 })])).toBe(false);
    expect(isContractIncomplete([spec({ direction: "Upper" })])).toBe(true);
    expect(isContractIncomplete([spec({ direction: "Upper", max: Number.NaN })])).toBe(true);
    expect(isContractIncomplete([spec({ direction: "Lower", min: 75 })])).toBe(false);
    expect(isContractIncomplete([spec({ direction: "Lower" })])).toBe(true);
    // 残留的另一端不算数: 上限型带着下限也完整
    expect(isContractIncomplete([spec({ direction: "Upper", min: 3, max: 10 })])).toBe(false);
  });

  it("区间型两端都要, 且下限不能超过上限", () => {
    expect(isContractIncomplete([spec({ min: 1.2, max: 1.4 })])).toBe(false);
    expect(isContractIncomplete([spec({ min: 1.2 })])).toBe(true);
    expect(isContractIncomplete([spec({ max: 1.4 })])).toBe(true);
    expect(isContractIncomplete([spec({ min: 1.5, max: 1.4 })])).toBe(true);
  });

  it("停用的项核心不看, 缺边界也不算不完整", () => {
    expect(isContractIncomplete([spec({ direction: "Upper", enabled: false })])).toBe(false);
  });

  it("空合同没有缺项", () => {
    expect(isContractIncomplete([])).toBe(false);
  });
});

describe("发给核心前按方向丢掉用不到的界", () => {
  const base = { indicator: "S", enabled: true } as unknown as Spec;

  it("上限型丢掉残留的下限, 下限型丢掉残留的上限", () => {
    expect(withoutUnusedBound({ ...base, direction: "Upper", min: 3, max: 1 })).toMatchObject({
      min: null,
      max: 1,
    });
    expect(withoutUnusedBound({ ...base, direction: "Lower", min: 80, max: 60 })).toMatchObject({
      min: 80,
      max: null,
    });
  });

  it("区间型两端都留; 没有残留值的原样返回", () => {
    const range = { ...base, direction: "Range", min: 24, max: 28 } as Spec;
    expect(withoutUnusedBound(range)).toBe(range);
    const upper = { ...base, direction: "Upper", min: null, max: 1 } as Spec;
    expect(withoutUnusedBound(upper)).toBe(upper);
  });
});
