/**
 * 云端同步决策纯函数的穷尽表 (cloudStorage.ts 的 decideInit / decidePersist / decideWrite).
 *
 * 每格都写明期望动作: 改决策函数就是改这张表, 对不上先想清是改错了还是表该改.
 * 末尾一组是"已知没覆盖的现状" (见 cloudStorage.ts 文件头的 4 条): 这些断言固定的不是
 * 正确行为, 而是现在会怎么走 —— 修的时候连这几条断言一起改.
 */
import { describe, expect, it } from "vitest";
import {
  decideInit,
  decidePersist,
  decideWrite,
  type InitAction,
  type PersistAction,
  type WriteAction,
} from "./cloudStorage";

// ---------------------------------------------------------------------------
// decideInit: initialized × pending × 本机=云端? × 基线 全组合 (2×3×2×3 = 36 格)
// ---------------------------------------------------------------------------
type InitRow = [
  initialized: boolean,
  pending: "none" | "dirty" | "conflict",
  same: boolean,
  baseline: "none" | "remote" | "other",
  expected: InitAction,
];

const INIT_TABLE: InitRow[] = [
  // 云端没初始化: 一律把本机整份推上去当第一份, 其余输入不影响.
  [false, "none", false, "none", "seed_cloud"],
  [false, "none", false, "remote", "seed_cloud"],
  [false, "none", false, "other", "seed_cloud"],
  [false, "none", true, "none", "seed_cloud"],
  [false, "none", true, "remote", "seed_cloud"],
  [false, "none", true, "other", "seed_cloud"],
  [false, "dirty", false, "none", "seed_cloud"],
  [false, "dirty", false, "remote", "seed_cloud"],
  [false, "dirty", false, "other", "seed_cloud"],
  [false, "dirty", true, "none", "seed_cloud"],
  [false, "dirty", true, "remote", "seed_cloud"],
  [false, "dirty", true, "other", "seed_cloud"],
  [false, "conflict", false, "none", "seed_cloud"],
  [false, "conflict", false, "remote", "seed_cloud"],
  [false, "conflict", false, "other", "seed_cloud"],
  [false, "conflict", true, "none", "seed_cloud"],
  [false, "conflict", true, "remote", "seed_cloud"],
  [false, "conflict", true, "other", "seed_cloud"],
  // 没有未同步标记: 用云端覆盖本机 (不管本机云端是否相同、有没有基线).
  [true, "none", false, "none", "adopt_cloud"],
  [true, "none", false, "remote", "adopt_cloud"],
  [true, "none", false, "other", "adopt_cloud"],
  [true, "none", true, "none", "adopt_cloud"],
  [true, "none", true, "remote", "adopt_cloud"],
  [true, "none", true, "other", "adopt_cloud"],
  // 有待写标记, 本机与云端相同: 上次其实写成功了, 只清标记记基线.
  [true, "dirty", true, "none", "already_in_sync"],
  [true, "dirty", true, "remote", "already_in_sync"],
  [true, "dirty", true, "other", "already_in_sync"],
  // 有待写标记, 本机比云端新: 云端还停在基线上就推本机, 否则算冲突.
  [true, "dirty", false, "none", "conflict"],
  [true, "dirty", false, "remote", "push_local"],
  [true, "dirty", false, "other", "conflict"],
  // 冲突标记 (上次冲突没选完): 与待写标记同一条路径 (现状如此).
  [true, "conflict", true, "none", "already_in_sync"],
  [true, "conflict", true, "remote", "already_in_sync"],
  [true, "conflict", true, "other", "already_in_sync"],
  [true, "conflict", false, "none", "conflict"],
  [true, "conflict", false, "remote", "push_local"],
  [true, "conflict", false, "other", "conflict"],
];

describe("decideInit", () => {
  for (const [initialized, pending, same, baseline, expected] of INIT_TABLE) {
    it(
      `initialized=${initialized} pending=${pending} 本机${same ? "=" : "≠"}云端 ` +
        `baseline=${baseline} → ${expected}`,
      () => {
        expect(
          decideInit({
            initialized,
            pending: pending === "none" ? null : pending === "dirty" ? "1" : "conflict",
            baseline: baseline === "none" ? null : baseline === "remote" ? "R" : "B",
            local: same ? "R" : "L",
            remote: "R",
          }),
        ).toBe(expected);
      },
    );
  }

  // 上表的 baseline="other" 取的是与本机、云端都不同的值。下面补另一种相等关系: 本机 == 基线 ≠ 云端
  // (本机其实没改, 是别人改了云端)。现在的实现不比 "本机 vs 基线", 所以结果只看标记 —— 以后改成比它时这里会报警。
  it("本机 == 基线 ≠ 云端: 没有待写标记就用云端", () => {
    expect(
      decideInit({ initialized: true, pending: null, baseline: "L", local: "L", remote: "R" }),
    ).toBe("adopt_cloud");
  });
  it("本机 == 基线 ≠ 云端: 有待写标记仍判冲突 (现状)", () => {
    expect(
      decideInit({ initialized: true, pending: "1", baseline: "L", local: "L", remote: "R" }),
    ).toBe("conflict");
  });
});

// ---------------------------------------------------------------------------
// decidePersist: conflict × pending × inFlight × next=baseline? 全组合 (2×3×2×2 = 24 格)
// ---------------------------------------------------------------------------
type PersistRow = [
  conflict: boolean,
  pending: "none" | "dirty" | "conflict",
  inFlight: number,
  same: boolean,
  expected: PersistAction,
];

const PERSIST_TABLE: PersistRow[] = [
  // 本标签页冲突没选完: 改动只留在本机, 其余输入不影响.
  [true, "none", 0, false, "hold"],
  [true, "none", 0, true, "hold"],
  [true, "none", 1, false, "hold"],
  [true, "none", 1, true, "hold"],
  [true, "dirty", 0, false, "hold"],
  [true, "dirty", 0, true, "hold"],
  [true, "dirty", 1, false, "hold"],
  [true, "dirty", 1, true, "hold"],
  [true, "conflict", 0, false, "hold"],
  [true, "conflict", 0, true, "hold"],
  [true, "conflict", 1, false, "hold"],
  [true, "conflict", 1, true, "hold"],
  // 别的标签页标了冲突: 这边也进冲突状态.
  [false, "conflict", 0, false, "join_conflict"],
  [false, "conflict", 0, true, "join_conflict"],
  [false, "conflict", 1, false, "join_conflict"],
  [false, "conflict", 1, true, "join_conflict"],
  // 没有冲突: 内容与基线一致且无在途写回才算一致; 否则排一笔写回.
  [false, "none", 0, true, "in_sync"],
  [false, "none", 0, false, "write"],
  [false, "none", 1, true, "write"],
  [false, "none", 1, false, "write"],
  [false, "dirty", 0, true, "in_sync"],
  [false, "dirty", 0, false, "write"],
  [false, "dirty", 1, true, "write"],
  [false, "dirty", 1, false, "write"],
];

describe("decidePersist", () => {
  for (const [conflict, pending, inFlight, same, expected] of PERSIST_TABLE) {
    it(
      `conflict=${conflict} pending=${pending} inFlight=${inFlight} next${same ? "=" : "≠"}baseline → ${expected}`,
      () => {
        expect(
          decidePersist({
            conflict,
            pending: pending === "none" ? null : pending === "dirty" ? "1" : "conflict",
            inFlight,
            next: same ? "R" : "N",
            baseline: "R",
          }),
        ).toBe(expected);
      },
    );
  }

  it("没有基线 (null) 时 next 永远不等于 baseline: 一律 write", () => {
    expect(decidePersist({ conflict: false, pending: null, inFlight: 0, next: "N", baseline: null }))
      .toBe("write");
  });
});

// ---------------------------------------------------------------------------
// decideWrite: 轮到写回时先拉云端再定 (remote / next / baseline 三方关系, 6 格)
// ---------------------------------------------------------------------------
type WriteRow = [remote: string, next: string, baseline: string | null, expected: WriteAction];

const WRITE_TABLE: WriteRow[] = [
  // 云端已是要写的内容: 不用发请求, 只推进基线 (三个相同也走这条, 顺序在前).
  ["R", "R", "R", "already_written"],
  ["R", "R", "B", "already_written"],
  ["R", "R", null, "already_written"],
  // 云端还停在基线上: 推上去.
  ["R", "N", "R", "push"],
  // 云端既不是要写的内容也不在基线上: 被别人改过, 不推.
  ["R", "N", "B", "conflict"],
  ["R", "N", null, "conflict"],
  // 要写的内容等于基线 (在途期间用户把改动改回了原值), 而云端被别人改过: 仍是冲突, 不能当成 "没变化" 放过。
  ["R", "N", "N", "conflict"],
];

describe("decideWrite", () => {
  for (const [remote, next, baseline, expected] of WRITE_TABLE) {
    it(`remote=${remote} next=${next} baseline=${baseline} → ${expected}`, () => {
      expect(decideWrite({ remote, next, baseline })).toBe(expected);
    });
  }
});

// ---------------------------------------------------------------------------
// 已知没覆盖的现状 (cloudStorage.ts 文件头的 4 条): 固定的是"现在会这样走", 不是"应该这样".
// 第 3 条 (多标签页事件不互通) 与第 4 条 (重新初始化不取消在途写回) 在外壳, 纯函数覆盖不到.
// ---------------------------------------------------------------------------
describe("已知没覆盖的现状 (修的时候连这些断言一起改)", () => {
  it("1. 写成功但回包丢了, 之后再改一笔: 把自己写的上一笔误报成冲突", () => {
    // remote 其实是自己上一笔写的内容, 但它既不是这一笔也不在基线上, 只能判冲突.
    expect(decideWrite({ remote: "自己上一笔", next: "新改动", baseline: "更早的基线" }))
      .toBe("conflict");
  });

  it("2. 两台设备同时首次登录: 都判 seed_cloud, 后到覆盖先到、没有任何提示", () => {
    expect(
      decideInit({ initialized: false, pending: null, baseline: null, local: "设备B", remote: "空库" }),
    ).toBe("seed_cloud");
  });
});
