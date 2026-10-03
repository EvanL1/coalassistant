import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  addOverrides,
  deleteOverride,
  getCalibration,
  getCsrModel,
  getSettings,
  listOverrides,
  setRankInteractionK,
} from "./admin";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

beforeEach(() => {
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("后台设置", () => {
  it("读取设置走同源 Cookie, 不缓存", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ ok: true, settings: { rank_interaction_k: 12.5 } }),
    );

    await expect(getSettings()).resolves.toEqual({ rank_interaction_k: 12.5 });
    expect(fetch).toHaveBeenCalledWith("/api/admin/settings", {
      credentials: "same-origin",
      cache: "no-store",
    });
  });

  it("关闭 k 时发送 null 而不是省略字段", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ ok: true, settings: { rank_interaction_k: null } }),
    );

    await expect(setRankInteractionK(null)).resolves.toEqual({ rank_interaction_k: null });
    expect(fetch).toHaveBeenCalledWith("/api/admin/settings", {
      credentials: "same-origin",
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: '{"rank_interaction_k":null}',
    });
  });

  it("非管理员的 403 理由原样抛出", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ ok: false, reason: "需要管理员权限" }, 403),
    );

    await expect(getSettings()).rejects.toThrow("需要管理员权限");
  });
});

describe("校准与模型状态", () => {
  it("解包 calibration 与 model", async () => {
    const calibration = {
      n: 3,
      skipped: 1,
      k: 10,
      intercept: 0.5,
      k_std_error: 2,
      d_min: 0.1,
      d_max: 0.4,
      recommended: true,
      reason: "ok",
      points: [],
    };
    vi.mocked(fetch)
      .mockResolvedValueOnce(jsonResponse({ ok: true, calibration }))
      .mockResolvedValueOnce(
        jsonResponse({
          ok: true,
          model: { samples: 4, required: 30, ready: false, r_squared: null },
        }),
      );

    await expect(getCalibration()).resolves.toEqual(calibration);
    await expect(getCsrModel()).resolves.toEqual({
      samples: 4,
      required: 30,
      ready: false,
      r_squared: null,
    });
    expect(vi.mocked(fetch).mock.calls.map((c) => c[0])).toEqual([
      "/api/admin/calibration",
      "/api/admin/csr-model",
    ]);
  });
});

describe("煤库覆盖层", () => {
  it("列表缺 overrides 字段时返回空数组", async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ ok: true }));
    await expect(listOverrides()).resolves.toEqual([]);
    expect(fetch).toHaveBeenCalledWith("/api/master/overrides", {
      credentials: "same-origin",
      cache: "no-store",
    });
  });

  it("新增按 updates 批量格式提交", async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ ok: true, applied: 1 }));
    const update = {
      coal: "临北",
      field: "CSR" as const,
      value: 62,
      source: "化验单 A1",
      confidence: "high" as const,
    };

    await addOverrides([update]);
    expect(fetch).toHaveBeenCalledWith("/api/master/coals", {
      credentials: "same-origin",
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ updates: [update] }),
    });
  });

  it("422 校验失败把 errors 列表拼成理由", async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ ok: false, errors: ["临北.CSR 基线已有值", "来源不能为空"] }, 422),
    );

    await expect(
      addOverrides([
        { coal: "临北", field: "CSR", value: 1, source: "", confidence: "low" },
      ]),
    ).rejects.toThrow("临北.CSR 基线已有值; 来源不能为空");
  });

  it("删除对煤种和指标都做 URL 编码", async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ ok: true }));

    await deleteOverride("山西/焦 煤", "CSR");
    expect(fetch).toHaveBeenCalledWith(
      "/api/master/coals/%E5%B1%B1%E8%A5%BF%2F%E7%84%A6%20%E7%85%A4/CSR",
      { credentials: "same-origin", method: "DELETE" },
    );
  });
});
