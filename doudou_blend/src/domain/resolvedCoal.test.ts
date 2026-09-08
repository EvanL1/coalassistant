import { describe, expect, it } from "vitest";
import type { CoalPref } from "../storage";
import type { MasterCoalEntry } from "../types";
import {
  resolveCoal,
  resolveCoalPool,
  summarizeDrift,
  toBlendCoal,
} from "./resolvedCoal";

const masterCoal: MasterCoalEntry = {
  name: "测试主煤",
  region: "山西",
  coal_type: "焦煤",
  status: "verified",
  props: { S: 0.5, A: 10, G: 80 },
  fob: 1_000,
  frt: 100,
};

describe("resolveCoal", () => {
  it("无覆盖时展示值和求解值都使用 Master", () => {
    const resolved = resolveCoal(masterCoal, null, "master");

    expect(resolved).toMatchObject({
      origin: "master",
      props: masterCoal.props,
      fob: 1_000,
      frt: 100,
      cif: 1_100,
      requestedEnabled: true,
      effectiveEnabled: true,
      readiness: "ready",
      hasOverrides: false,
    });
    expect(toBlendCoal(resolved)).toEqual({
      name: "测试主煤",
      props: masterCoal.props,
      fob: 1_000,
      frt: 100,
    });
  });

  it("价格和指标覆盖在展示与求解输入中保持一致，并保留合法的 0", () => {
    const pref: CoalPref = {
      enabled: true,
      fob_override: 0,
      frt_override: 80,
      props_override: { S: 0, G: 85 },
    };

    const resolved = resolveCoal(masterCoal, pref, "master");

    expect(resolved).toMatchObject({
      fob: 0,
      frt: 80,
      cif: 80,
      props: { S: 0, A: 10, G: 85 },
      hasOverrides: true,
      overriddenProps: { S: true, G: true },
      readiness: "ready",
    });
    expect(toBlendCoal(resolved)).toMatchObject({
      fob: 0,
      frt: 80,
      props: { S: 0, A: 10, G: 85 },
    });
  });

  it("null 覆盖明确表示继承 Master", () => {
    const resolved = resolveCoal(
      masterCoal,
      {
        enabled: true,
        fob_override: null,
        frt_override: null,
      },
      "master",
    );

    expect(resolved.fob).toBe(1_000);
    expect(resolved.frt).toBe(100);
    expect(resolved.hasOverrides).toBe(false);
  });

  it("隐藏状态一票否决，不进入求解", () => {
    const resolved = resolveCoal(
      masterCoal,
      { enabled: true, hidden: true },
      "master",
    );

    expect(resolved.requestedEnabled).toBe(true);
    expect(resolved.effectiveEnabled).toBe(false);
    expect(resolved.readiness).toBe("hidden");
    expect(toBlendCoal(resolved)).toBeNull();
  });

  it("用户新增煤可由覆盖值补齐并进入求解", () => {
    const userCoal: MasterCoalEntry = {
      name: "用户煤",
      status: "draft",
      props: {},
      fob: null,
      frt: null,
    };
    const resolved = resolveCoal(
      userCoal,
      {
        enabled: true,
        fob_override: 900,
        frt_override: 120,
        props_override: { S: 0.6, A: 9.5 },
      },
      "user",
    );

    expect(resolved).toMatchObject({
      origin: "user",
      fob: 900,
      frt: 120,
      props: { S: 0.6, A: 9.5 },
      readiness: "ready",
    });
    expect(toBlendCoal(resolved)?.name).toBe("用户煤");
  });

  it("启用但缺少价格时给出明确原因并排除求解", () => {
    const incomplete: MasterCoalEntry = {
      ...masterCoal,
      fob: null,
      frt: null,
    };

    const missingFob = resolveCoal(
      incomplete,
      { enabled: true, frt_override: 100 },
      "master",
    );
    const missingFrt = resolveCoal(
      incomplete,
      { enabled: true, fob_override: 1_000 },
      "master",
    );

    expect(missingFob.readiness).toBe("missing_fob");
    expect(missingFrt.readiness).toBe("missing_frt");
    expect(toBlendCoal(missingFob)).toBeNull();
    expect(toBlendCoal(missingFrt)).toBeNull();
  });

  it("非法持久化覆盖不会静默回退 Master 或进入求解", () => {
    const invalidPref = {
      enabled: true,
      fob_override: "坏数据",
    } as unknown as CoalPref;

    const resolved = resolveCoal(masterCoal, invalidPref, "master");

    expect(resolved.readiness).toBe("invalid_override");
    expect(toBlendCoal(resolved)).toBeNull();
  });

  it("未知指标键会标记为非法且不会传给求解器", () => {
    const resolved = resolveCoal(
      masterCoal,
      {
        enabled: true,
        props_override: { typo: 12 },
      },
      "master",
    );

    expect(resolved.readiness).toBe("invalid_override");
    expect(resolved.props).not.toHaveProperty("typo");
    expect(toBlendCoal(resolved)).toBeNull();
  });

  it("价格相加溢出时不会生成无限 CIF", () => {
    const resolved = resolveCoal(
      {
        ...masterCoal,
        fob: Number.MAX_VALUE,
        frt: Number.MAX_VALUE,
      },
      null,
      "master",
    );

    expect(resolved.cif).toBeNull();
    expect(resolved.readiness).toBe("invalid_override");
    expect(toBlendCoal(resolved)).toBeNull();
  });

  it("未启用煤保留有效展示值但不进入求解", () => {
    const resolved = resolveCoal(
      masterCoal,
      { enabled: false },
      "master",
    );

    expect(resolved.fob).toBe(1_000);
    expect(resolved.readiness).toBe("disabled");
    expect(toBlendCoal(resolved)).toBeNull();
  });
});

describe("resolveCoalPool", () => {
  it("统一标记 Master 与用户煤来源，并让用户煤优先展示", () => {
    const userCoal: MasterCoalEntry = {
      name: "用户煤",
      status: "draft",
      props: {},
    };

    const resolved = resolveCoalPool(
      [masterCoal],
      [userCoal],
      {
        用户煤: {
          enabled: false,
        },
      },
    );

    expect(resolved.map(({ name, origin }) => ({ name, origin }))).toEqual([
      { name: "用户煤", origin: "user" },
      { name: "测试主煤", origin: "master" },
    ]);
  });

  it("归一化重名煤全部标记冲突并排除求解", () => {
    const resolved = resolveCoalPool(
      [masterCoal],
      [
        {
          ...masterCoal,
          name: "　测试主煤 ",
          status: "draft",
        },
      ],
      {
        "　测试主煤 ": { enabled: true },
      },
    );

    expect(resolved.map((coal) => coal.readiness)).toEqual([
      "duplicate_name",
      "duplicate_name",
    ]);
    expect(resolved.map(toBlendCoal)).toEqual([null, null]);
  });
});

describe("价格漂移推算", () => {
  /** 临北 6-30 报 1425, 9-8 复录 1650 -> 全池比例 1.157894... */
  const drift = {
    anchor: {
      coals: ["临北"],
      quotes: {
        临北: [
          { date: "2026-06-30", fob: 1425 },
          { date: "2026-09-08", fob: 1650 },
        ],
      },
    },
    masterUpdatedAt: "2026-06-30",
  };
  const ratio = 1650 / 1425;

  it("没有锚点时行为与从前完全一致", () => {
    const resolved = resolveCoal(masterCoal, null, "master");
    expect(resolved.fob_drifted).toBeNull();
    expect(resolved.drift_ratio).toBeNull();
    expect(resolved.cif).toBe(1_100);
    expect(toBlendCoal(resolved)?.fob).toBe(1_000);
  });

  it("没单独录过价的煤按 master updated_at 当录价日推算", () => {
    const resolved = resolveCoal(masterCoal, null, "master", drift);
    expect(resolved.fob_quoted_at).toBe("2026-06-30");
    expect(resolved.fob).toBe(1_000);
    expect(resolved.fob_drifted).toBeCloseTo(1_000 * ratio, 6);
    expect(resolved.drift_ratio).toBeCloseTo(ratio, 6);
  });

  it("到厂价用推算价, 运费不参与漂移", () => {
    const resolved = resolveCoal(masterCoal, null, "master", drift);
    expect(resolved.cif).toBeCloseTo(1_000 * ratio + 100, 6);
  });

  it("求解器拿到的是推算价而不是旧报价", () => {
    const resolved = resolveCoal(masterCoal, null, "master", drift);
    expect(toBlendCoal(resolved)).toMatchObject({
      fob: resolved.fob_drifted,
      frt: 100,
    });
  });

  it("刚录过价的煤不再被漂移", () => {
    const pref: CoalPref = {
      fob_override: 1_200,
      fob_quoted_at: "2026-09-08",
    };
    const resolved = resolveCoal(masterCoal, pref, "master", drift);
    expect(resolved.fob).toBe(1_200);
    expect(resolved.fob_drifted).toBe(1_200);
    expect(resolved.cif).toBe(1_300);
  });

  it("旧数据没有 fob_quoted_at 时退回 updated_at 时间戳", () => {
    const pref: CoalPref = {
      fob_override: 1_200,
      updated_at: "2026-09-08T02:00:00.000Z",
    };
    const resolved = resolveCoal(masterCoal, pref, "master", drift);
    expect(resolved.fob_quoted_at).toBe("2026-09-08");
    expect(resolved.fob_drifted).toBe(1_200);
  });

  it("锚点定位不到基准时不推算, 退回录入价求解", () => {
    const pref: CoalPref = {
      fob_override: 1_200,
      fob_quoted_at: "2026-01-01",
    };
    const resolved = resolveCoal(masterCoal, pref, "master", drift);
    expect(resolved.fob_drifted).toBeNull();
    expect(resolved.cif).toBe(1_300);
    expect(toBlendCoal(resolved)?.fob).toBe(1_200);
  });

  it("整池解析时漂移逐煤生效", () => {
    const other: MasterCoalEntry = { ...masterCoal, name: "另一个煤", fob: 800 };
    const pool = resolveCoalPool([masterCoal, other], [], {}, drift);
    expect(pool.map((c) => c.fob_drifted)).toEqual([
      expect.closeTo(1_000 * ratio, 6),
      expect.closeTo(800 * ratio, 6),
    ]);
  });
});

describe("summarizeDrift", () => {
  const drift = {
    anchor: {
      coals: ["临北"],
      quotes: {
        临北: [
          { date: "2026-06-30", fob: 1425 },
          { date: "2026-09-08", fob: 1650 },
        ],
      },
    },
    masterUpdatedAt: "2026-06-30",
  };

  it("没有锚点时不生成摘要", () => {
    const pool = resolveCoalPool([masterCoal], [], {});
    expect(summarizeDrift(pool, [])).toBeNull();
  });

  it("统计被推算的煤数量与平均比例", () => {
    const other: MasterCoalEntry = { ...masterCoal, name: "另一个煤", fob: 800 };
    const pool = resolveCoalPool([masterCoal, other], [], {}, drift);
    const summary = summarizeDrift(pool, drift.anchor.coals);
    expect(summary).toMatchObject({
      count: 2,
      oldestQuotedAt: "2026-06-30",
      anchors: ["临北"],
    });
    expect(summary?.avgRatio).toBeCloseTo(1650 / 1425, 6);
  });

  it("刚录过价的煤不计入推算", () => {
    const pool = resolveCoalPool(
      [masterCoal],
      [],
      { 测试主煤: { fob_override: 1_200, fob_quoted_at: "2026-09-08" } },
      drift,
    );
    expect(summarizeDrift(pool, drift.anchor.coals)).toBeNull();
  });

  it("停用的煤不计入推算", () => {
    const pool = resolveCoalPool(
      [masterCoal],
      [],
      { 测试主煤: { enabled: false } },
      drift,
    );
    expect(summarizeDrift(pool, drift.anchor.coals)).toBeNull();
  });
});
