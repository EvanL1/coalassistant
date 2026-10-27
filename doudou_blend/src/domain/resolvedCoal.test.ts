import { describe, expect, it } from "vitest";
import type { CoalPref } from "../storage";
import type { MasterCoalEntry, PurchaseTerms } from "../types";
import { resolveCoal, resolveCoalPool, toEstimateCoal } from "./resolvedCoal";

const masterCoal: MasterCoalEntry = {
  name: "测试主煤",
  region: "山西",
  coal_type: "焦煤",
  status: "verified",
  props: { S: 0.5, A: 10, G: 80 },
  fob: null,
  frt: null,
};

describe("resolveCoal", () => {
  it("启用的煤一律按估算价进求解: 不带报价, 煤种/地区随煤走", () => {
    const resolved = resolveCoal(masterCoal, null, "master");

    expect(resolved).toMatchObject({
      origin: "master",
      props: masterCoal.props,
      requestedEnabled: true,
      effectiveEnabled: true,
      readiness: "ready",
      hasOverrides: false,
    });
    expect(toEstimateCoal(resolved)).toEqual({
      name: "测试主煤",
      props: masterCoal.props,
      fob: 0,
      frt: 0,
      // 煤种、地区随煤进求解请求: 估价按 煤种 → 形态 → 地区 对标, 今日屏按煤种展示.
      coal_type: "焦煤",
      region: "山西",
    });
    expect(
      toEstimateCoal(resolveCoal({ ...masterCoal, form: "筛选精煤" }, null, "master"))?.form,
    ).toBe("筛选精煤");
  });

  it("master 里残留的出厂价/运费不进求解", () => {
    const resolved = resolveCoal({ ...masterCoal, fob: 1_000, frt: 100 }, null, "master");

    expect(resolved.readiness).toBe("ready");
    expect(toEstimateCoal(resolved)).toMatchObject({ fob: 0, frt: 0 });
  });

  it("旧版本录过的价格键被忽略, 不算修改也不算坏数据", () => {
    const legacy = {
      enabled: true,
      fob_override: 1_200,
      frt_override: 24,
      is_price_anchor: true,
      fob_history: [{ date: "2026-09-01", fob: 1_150 }],
    } as unknown as CoalPref;

    const resolved = resolveCoal(masterCoal, legacy, "master");

    expect(resolved.readiness).toBe("ready");
    expect(resolved.hasOverrides).toBe(false);
    expect(toEstimateCoal(resolved)).toMatchObject({ fob: 0, frt: 0 });
  });

  it("指标覆盖进入求解输入, 并保留合法的 0", () => {
    const resolved = resolveCoal(
      masterCoal,
      { enabled: true, props_override: { S: 0, G: 85 } },
      "master",
    );

    expect(resolved).toMatchObject({
      props: { S: 0, A: 10, G: 85 },
      hasOverrides: true,
      overriddenProps: { S: true, G: true },
      readiness: "ready",
    });
    expect(toEstimateCoal(resolved)?.props).toEqual({ S: 0, A: 10, G: 85 });
  });

  it("已停用 (archived) 的煤一票否决, 用户偏好里的启用也拉不回来", () => {
    const resolved = resolveCoal(
      { ...masterCoal, status: "archived" },
      { enabled: true },
      "master",
    );

    expect(resolved.effectiveEnabled).toBe(false);
    expect(resolved.readiness).toBe("disabled");
    expect(toEstimateCoal(resolved)).toBeNull();
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
    expect(toEstimateCoal(resolved)).toBeNull();
  });

  it("用户新增煤补齐指标后进入求解", () => {
    const userCoal: MasterCoalEntry = {
      name: "用户煤",
      status: "draft",
      props: {},
      fob: null,
      frt: null,
    };
    const resolved = resolveCoal(
      userCoal,
      { enabled: true, props_override: { S: 0.6, A: 9.5 } },
      "user",
    );

    expect(resolved).toMatchObject({
      origin: "user",
      props: { S: 0.6, A: 9.5 },
      readiness: "ready",
    });
    expect(toEstimateCoal(resolved)?.name).toBe("用户煤");
  });

  it("非法持久化覆盖不会静默回退 Master 或进入求解", () => {
    const invalidPref = {
      enabled: "坏数据",
    } as unknown as CoalPref;

    const resolved = resolveCoal(masterCoal, invalidPref, "master");

    expect(resolved.readiness).toBe("disabled");
    expect(toEstimateCoal(resolved)).toBeNull();

    const badProps = resolveCoal(
      masterCoal,
      { enabled: true, props_override: { S: -1 } },
      "master",
    );
    expect(badProps.readiness).toBe("invalid_override");
    expect(toEstimateCoal(badProps)).toBeNull();
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
    expect(toEstimateCoal(resolved)).toBeNull();
  });

  it("未启用煤保留展示值但不进入求解", () => {
    const resolved = resolveCoal(
      masterCoal,
      { enabled: false },
      "master",
    );

    expect(resolved.props).toEqual(masterCoal.props);
    expect(resolved.readiness).toBe("disabled");
    expect(toEstimateCoal(resolved)).toBeNull();
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
    expect(resolved.map((coal) => toEstimateCoal(coal))).toEqual([null, null]);
  });
});

describe("toEstimateCoal 的 purchaseTerms 参数 (调用方传入, 不在这里重算)", () => {
  const terms: PurchaseTerms = {
    clauses: [{ indicator: "S", guarantee: 0.6, direction: "Upper", tiers: [] }],
  } as unknown as PurchaseTerms;

  it("传入 terms 时挂到 purchase_terms 上 (核心并入煤池时随煤走)", () => {
    const resolved = resolveCoal(masterCoal, null, "master");
    expect(toEstimateCoal(resolved, terms)?.purchase_terms).toBe(terms);
  });

  it("不传或传 null 时不产出 purchase_terms 字段", () => {
    const resolved = resolveCoal(masterCoal, null, "master");
    expect(toEstimateCoal(resolved)?.purchase_terms).toBeUndefined();
    expect(toEstimateCoal(resolved, null)?.purchase_terms).toBeUndefined();
  });

  it("非 ready 的煤仍然返回 null, 不受 purchaseTerms 参数影响", () => {
    const resolved = resolveCoal(masterCoal, { enabled: false }, "master");
    expect(toEstimateCoal(resolved, terms)).toBeNull();
  });
});
