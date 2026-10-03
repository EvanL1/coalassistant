// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Calibration } from "../admin";
import { AdminScreen } from "./AdminScreen";

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  setRankInteractionK: vi.fn(),
  getCalibration: vi.fn(),
  getCsrModel: vi.fn(),
  listOverrides: vi.fn(),
  addOverrides: vi.fn(),
  deleteOverride: vi.fn(),
  loadMaster: vi.fn(),
  invalidateMaster: vi.fn(),
  logout: vi.fn(),
  confirm: vi.fn(),
}));

vi.mock("../admin", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../admin")>()),
  getSettings: mocks.getSettings,
  setRankInteractionK: mocks.setRankInteractionK,
  getCalibration: mocks.getCalibration,
  getCsrModel: mocks.getCsrModel,
  listOverrides: mocks.listOverrides,
  addOverrides: mocks.addOverrides,
  deleteOverride: mocks.deleteOverride,
}));

vi.mock("../master_loader", () => ({
  loadMaster: mocks.loadMaster,
  invalidateMaster: mocks.invalidateMaster,
}));

vi.mock("../auth", () => ({ logout: mocks.logout }));

vi.mock("../apiKeys", () => ({
  listApiKeys: vi.fn().mockResolvedValue([]),
  createApiKey: vi.fn(),
  revokeApiKey: vi.fn(),
}));

const calibration: Calibration = {
  n: 12,
  skipped: 2,
  k: 18.4567,
  intercept: 0.3,
  k_std_error: 3.1,
  d_min: 0.0123,
  d_max: 0.25,
  recommended: true,
  reason: "样本足够, 误差可接受",
  points: [
    {
      id: "p1",
      occurred_at: "2026-09-01T00:00:00Z",
      base_csr: 66.04,
      measured_csr: 63.2,
      rank_variance: 0.1234,
    },
  ],
};

const override = {
  coal_name: "临北",
  field: "CSR",
  value: 62,
  source: "化验单 A1",
  confidence: "high",
  updated_at: "2026-09-20T00:00:00Z",
  updated_by: "张三",
};

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.getSettings.mockResolvedValue({ rank_interaction_k: null });
  mocks.setRankInteractionK.mockImplementation(async (k: number | null) => ({
    rank_interaction_k: k,
  }));
  mocks.getCalibration.mockResolvedValue(calibration);
  mocks.getCsrModel.mockResolvedValue({
    samples: 7,
    required: 30,
    ready: false,
    r_squared: null,
  });
  mocks.listOverrides.mockResolvedValue([override]);
  mocks.addOverrides.mockResolvedValue(undefined);
  mocks.deleteOverride.mockResolvedValue(undefined);
  mocks.loadMaster.mockResolvedValue({ coals: [{ name: "临北" }, { name: "柳林" }] });
  mocks.confirm.mockReturnValue(true);
  vi.stubGlobal("confirm", mocks.confirm);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("AdminScreen 煤阶交互 k", () => {
  it("未设置时显示未启用, 保存输入值", async () => {
    render(<AdminScreen />);
    await screen.findByText("当前: 未启用");

    fireEvent.change(screen.getByLabelText("煤阶交互 k"), { target: { value: "15" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByText("当前: k = 15");
    expect(mocks.setRankInteractionK).toHaveBeenCalledWith(15);
  });

  it("负数在前端拦下, 不发请求", async () => {
    render(<AdminScreen />);
    await screen.findByText("当前: 未启用");

    fireEvent.change(screen.getByLabelText("煤阶交互 k"), { target: { value: "-1" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByText("k 必须是 ≥ 0 的数字");
    expect(mocks.setRankInteractionK).not.toHaveBeenCalled();
  });

  it("关闭按钮写入 null", async () => {
    mocks.getSettings.mockResolvedValue({ rank_interaction_k: 20 });
    render(<AdminScreen />);
    await screen.findByText("当前: k = 20");

    fireEvent.click(screen.getByRole("button", { name: "关闭" }));

    await screen.findByText("当前: 未启用");
    expect(mocks.setRankInteractionK).toHaveBeenCalledWith(null);
  });

  it("展示拟合结果, 采用按钮写入取整后的 k, 不自动套用", async () => {
    render(<AdminScreen />);
    await screen.findByText("样本足够, 误差可接受");
    expect(screen.getByText("拟合 k = 18.5 ± 6.2 (±2σ)")).toBeTruthy();
    expect(screen.getByText("煤阶方差 D 范围: 0.0123 ~ 0.25")).toBeTruthy();
    expect(screen.getByText("66.0")).toBeTruthy();
    expect(mocks.setRankInteractionK).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "采用拟合值 k=18.5" }));

    await waitFor(() => expect(mocks.setRankInteractionK).toHaveBeenCalledWith(18.5));
  });

  it("不推荐时不显示采用按钮", async () => {
    mocks.getCalibration.mockResolvedValue({
      ...calibration,
      recommended: false,
      reason: "样本太少, 暂不推荐",
    });
    render(<AdminScreen />);
    await screen.findByText("样本太少, 暂不推荐");
    expect(screen.queryByRole("button", { name: /采用拟合值/ })).toBeNull();
  });
});

describe("AdminScreen CSR 回归模型", () => {
  it("显示回填进度", async () => {
    render(<AdminScreen />);
    await screen.findByText("已回填 7 / 需要 30 条");
    expect(screen.getByText("样本不足")).toBeTruthy();
  });
});

describe("AdminScreen 煤库数据维护", () => {
  it("新增后失效 master 缓存并刷新列表", async () => {
    render(<AdminScreen />);
    await screen.findByRole("option", { name: "柳林" });

    fireEvent.change(screen.getByLabelText("煤种"), { target: { value: "柳林" } });
    fireEvent.change(screen.getByLabelText("指标"), { target: { value: "G" } });
    fireEvent.change(screen.getByLabelText("数值"), { target: { value: "78" } });
    fireEvent.change(screen.getByLabelText("来源"), { target: { value: " 报告 B " } });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));

    await waitFor(() => expect(mocks.invalidateMaster).toHaveBeenCalledTimes(1));
    expect(mocks.addOverrides).toHaveBeenCalledWith([
      { coal: "柳林", field: "G", value: 78, source: "报告 B", confidence: "medium" },
    ]);
    expect(mocks.listOverrides).toHaveBeenCalledTimes(2);
  });

  it("来源为空时前端拦下", async () => {
    render(<AdminScreen />);
    await screen.findByRole("option", { name: "柳林" });

    fireEvent.change(screen.getByLabelText("数值"), { target: { value: "78" } });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));

    await screen.findByText(/来源必填/);
    expect(mocks.addOverrides).not.toHaveBeenCalled();
  });

  it("服务端拒绝时显示理由, 不失效缓存", async () => {
    mocks.addOverrides.mockRejectedValue(new Error("临北.CSR 基线已有值"));
    render(<AdminScreen />);
    await screen.findByRole("option", { name: "柳林" });

    fireEvent.change(screen.getByLabelText("数值"), { target: { value: "60" } });
    fireEvent.change(screen.getByLabelText("来源"), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));

    await screen.findByText("临北.CSR 基线已有值");
    expect(mocks.invalidateMaster).not.toHaveBeenCalled();
  });

  it("确认后删除一行覆盖值", async () => {
    render(<AdminScreen />);
    const row = (await screen.findByText("化验单 A1")).closest("tr")!;
    expect(within(row).getByText("张三")).toBeTruthy();

    fireEvent.click(within(row).getByRole("button", { name: "删除 临北 CSR" }));

    await waitFor(() => expect(mocks.deleteOverride).toHaveBeenCalledWith("临北", "CSR"));
    await waitFor(() => expect(mocks.invalidateMaster).toHaveBeenCalledTimes(1));
  });

  it("取消确认则不删除", async () => {
    mocks.confirm.mockReturnValue(false);
    render(<AdminScreen />);
    fireEvent.click(await screen.findByRole("button", { name: "删除 临北 CSR" }));
    expect(mocks.deleteOverride).not.toHaveBeenCalled();
  });
});

describe("AdminScreen 页面", () => {
  it("包含 API 密钥卡片与退出按钮", async () => {
    render(<AdminScreen />);
    expect(screen.getByText("豆哥配煤 · 后台")).toBeTruthy();
    await screen.findByText("数据更新密钥");

    fireEvent.click(screen.getByRole("button", { name: "退出登录" }));
    expect(mocks.logout).toHaveBeenCalledTimes(1);
  });
});
