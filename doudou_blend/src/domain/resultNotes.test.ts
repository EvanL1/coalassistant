import { describe, expect, it } from "vitest";
import {
  friendlySolverText,
  shovelRoundingMissed,
  shovelRoundingNotice,
} from "./resultNotes";

const order = (shovels?: number) => ({ coal: "甲", ratio: 1, tons: 3000, shovels });

describe("整铲没凑成", () => {
  it("带了一铲吨数却没有铲数才算", () => {
    expect(shovelRoundingMissed({ shovel_tons: 3 }, { ok: true, orders: [order()] })).toBe(true);
    expect(shovelRoundingMissed({ shovel_tons: 3 }, { ok: true, orders: [order(1000)] })).toBe(false);
    expect(shovelRoundingMissed({}, { ok: true, orders: [order()] })).toBe(false);
    expect(shovelRoundingMissed({ shovel_tons: 3 }, { ok: false, orders: [order()] })).toBe(false);
    expect(shovelRoundingMissed({ shovel_tons: 3 }, { ok: true, orders: [] })).toBe(false);
  });

  it("提示写明一铲吨数", () => {
    expect(shovelRoundingNotice(3)).toContain("一铲 3 吨");
  });
});

describe("不可行原因的显示", () => {
  it("LP 换成用户看得懂的说法", () => {
    expect(friendlySolverText("约束冲突, LP 不可行")).toBe("合同要求互相冲突, 现有的煤怎么配都达不到");
    expect(friendlySolverText("岩相修复后 LP 不可行，已保留候选方案")).toBe(
      "岩相修复后 配不出满足合同的方案，已保留候选方案",
    );
    expect(friendlySolverText("无可用煤")).toBe("无可用煤");
  });
});
