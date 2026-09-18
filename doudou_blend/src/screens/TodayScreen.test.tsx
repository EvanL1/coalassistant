// @vitest-environment jsdom

import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BlendResult, CoalMaster } from "../types";
import { TodayScreen } from "./TodayScreen";

const mocks = vi.hoisted(() => ({
  getBackend: vi.fn(),
  loadMaster: vi.fn(),
  setQuantity: vi.fn(),
  writeText: vi.fn(),
  fetchCoalIndexRatioSince: vi.fn(),
  quantity: { value: 3_700 },
}));

vi.mock("../coal_index", () => ({
  fetchCoalIndexRatioSince: mocks.fetchCoalIndexRatioSince,
}));

vi.mock("../backend", () => ({
  getBackend: mocks.getBackend,
}));

vi.mock("../master_loader", () => ({
  loadMaster: mocks.loadMaster,
}));

vi.mock("../storage", () => ({
  getCoalPrefs: () => ({}),
  getQuantity: () => mocks.quantity.value,
  getUserContract: () => null,
  getUserCoals: () => [],
  setQuantity: mocks.setQuantity,
}));

const master: CoalMaster = {
  version: "test",
  updated_at: "2026-07-17",
  description: "测试数据",
  default_contract: {
    name: "默认测试合同",
    specs: [],
  },
  coals: [
    {
      name: "测试煤",
      status: "verified",
      props: { S: 0.5 },
      fob: 900,
      frt: 100,
    },
  ],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function makeResult(cif: number, quantity: number): BlendResult {
  return {
    ok: true,
    recipe: { 测试煤: 1 },
    cost: {
      fob_per_ton: cif - 100,
      frt_per_ton: 100,
      cif_per_ton: cif,
      total_fob: (cif - 100) * quantity,
      total_frt: 100 * quantity,
      total_cif: cif * quantity,
      purchase_adjust_per_ton: 0,
      penalty_per_ton: 0,
      net_per_ton: cif,
      total_purchase_adjust: 0,
      total_penalty: 0,
      total_net: cif * quantity,
    },
    orders: [
      {
        coal: "测试煤",
        ratio: 1,
        tons: quantity,
        cif_amount: cif * quantity,
        cif_eff_per_ton: cif,
      },
    ],
    indicator_check: [],
    warnings: [],
    quality_status: "Estimated",
  };
}

afterEach(() => {
  cleanup();
});

beforeEach(() => {
  mocks.quantity.value = 3_700;
  mocks.getBackend.mockReset();
  mocks.loadMaster.mockReset();
  mocks.setQuantity.mockReset();
  mocks.writeText.mockReset();
  mocks.setQuantity.mockImplementation((quantity: number) => {
    mocks.quantity.value = quantity;
  });
  mocks.loadMaster.mockResolvedValue(master);
  mocks.writeText.mockResolvedValue(undefined);
  mocks.fetchCoalIndexRatioSince.mockReset();
  mocks.fetchCoalIndexRatioSince.mockResolvedValue(null);
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: mocks.writeText },
  });
});

describe("TodayScreen 求解快照", () => {
  it("并发求解乱序完成时只展示最后一次结果", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const backend = {
      solveJson: vi
        .fn()
        .mockImplementationOnce(() => first.promise)
        .mockImplementationOnce(() => second.promise),
      saveHistory: vi.fn(),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<TodayScreen onNavigate={vi.fn()} />);
    await waitFor(() => expect(backend.solveJson).toHaveBeenCalledTimes(1));

    act(() => {
      window.dispatchEvent(new CustomEvent("doudou:prefs_changed"));
    });
    await waitFor(() => expect(backend.solveJson).toHaveBeenCalledTimes(2));

    await act(async () => {
      second.resolve(JSON.stringify(makeResult(1_200, 3_700)));
      await second.promise;
    });
    await waitFor(() =>
      expect(screen.getByText("1200", { selector: ".cost-int" })).toBeTruthy(),
    );

    await act(async () => {
      first.resolve(JSON.stringify(makeResult(1_000, 3_700)));
      await first.promise;
    });

    expect(screen.getByText("1200", { selector: ".cost-int" })).toBeTruthy();
    expect(screen.queryByText("1000", { selector: ".cost-int" })).toBeNull();
  });

  it("吨数重算完成前禁止旧结果操作，完成后保存和导出都使用新快照", async () => {
    const refreshed = deferred<string>();
    const backend = {
      solveJson: vi
        .fn()
        .mockResolvedValueOnce(JSON.stringify(makeResult(1_000, 3_700)))
        .mockImplementationOnce(() => refreshed.promise),
      saveHistory: vi.fn().mockResolvedValue(undefined),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<TodayScreen onNavigate={vi.fn()} />);
    await screen.findByText("1000", { selector: ".cost-int" });

    const quantityInput = screen.getByLabelText("采购总吨数");
    fireEvent.change(quantityInput, { target: { value: "5000" } });
    const exportButton = screen.getByRole("button", { name: "导出订单" });
    const saveButton = screen.getByRole("button", { name: "保存方案" });
    expect((exportButton as HTMLButtonElement).disabled).toBe(true);
    expect((saveButton as HTMLButtonElement).disabled).toBe(true);

    fireEvent.blur(quantityInput);
    fireEvent.click(exportButton);
    fireEvent.click(saveButton);
    await waitFor(() => expect(backend.solveJson).toHaveBeenCalledTimes(2));
    expect(mocks.writeText).not.toHaveBeenCalled();
    expect(backend.saveHistory).not.toHaveBeenCalled();

    const request = JSON.parse(
      backend.solveJson.mock.calls[1][0] as string,
    ) as { total_quantity: number };
    expect(request.total_quantity).toBe(5_000);

    await act(async () => {
      refreshed.resolve(JSON.stringify(makeResult(980, 5_000)));
      await refreshed.promise;
    });
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "导出订单" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );

    fireEvent.click(screen.getByRole("button", { name: "导出订单" }));
    await waitFor(() => expect(mocks.writeText).toHaveBeenCalledTimes(1));
    expect(mocks.writeText.mock.calls[0][0]).toContain("总量 5000 吨");

    const currentSaveButton = screen.getByRole("button", { name: "保存方案" });
    fireEvent.click(currentSaveButton);
    fireEvent.click(currentSaveButton);
    await waitFor(() => expect(backend.saveHistory).toHaveBeenCalledTimes(1));
    expect(backend.saveHistory.mock.calls[0][2]).toBe(5_000);
  });

  it("吨数未变化时 blur 不会发起无意义重算或拦截导出", async () => {
    const backend = {
      solveJson: vi.fn().mockResolvedValue(
        JSON.stringify(makeResult(1_000, 3_700)),
      ),
      saveHistory: vi.fn(),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<TodayScreen onNavigate={vi.fn()} />);
    await screen.findByText("1000", { selector: ".cost-int" });

    fireEvent.blur(screen.getByLabelText("采购总吨数"));
    fireEvent.click(screen.getByRole("button", { name: "导出订单" }));

    await waitFor(() => expect(mocks.writeText).toHaveBeenCalledTimes(1));
    expect(backend.solveJson).toHaveBeenCalledTimes(1);
  });

  it("导出等待期间卸载，不会在卸载后新建反馈定时器", async () => {
    const clipboard = deferred<void>();
    mocks.writeText.mockImplementation(() => clipboard.promise);
    const backend = {
      solveJson: vi.fn().mockResolvedValue(
        JSON.stringify(makeResult(1_000, 3_700)),
      ),
      saveHistory: vi.fn(),
    };
    mocks.getBackend.mockResolvedValue(backend);

    const view = render(<TodayScreen onNavigate={vi.fn()} />);
    await screen.findByText("1000", { selector: ".cost-int" });
    const timerSpy = vi.spyOn(window, "setTimeout");
    timerSpy.mockClear();

    fireEvent.click(screen.getByRole("button", { name: "导出订单" }));
    view.unmount();
    await act(async () => {
      clipboard.resolve();
      await clipboard.promise;
    });

    expect(timerSpy).not.toHaveBeenCalled();
    timerSpy.mockRestore();
  });
});

describe("TodayScreen 报价时效提示", () => {
  /**
   * 回归: 早先只在"锚点推算生效后"才显示说明, 而真正危险的是没推算的默认态 ——
   * 卡片印着两位小数的到厂价和总额, 底下是几十天前的报价, 界面却一言不发.
   */
  it("没有锚点推算时也必须显示报价日与未校正提示", async () => {
    mocks.getBackend.mockResolvedValue({
      solveJson: vi.fn().mockResolvedValue(JSON.stringify(makeResult(1_000, 3_700))),
      saveHistory: vi.fn(),
    });

    render(<TodayScreen onNavigate={vi.fn()} />);
    await screen.findByText("1000", { selector: ".cost-int" });

    const note = await screen.findByText(/报价停留在 2026-07-17/);
    expect(note.textContent).toContain("未按市场校正");
    expect(note.className).toContain("cost-warn");
  });

  it("拿到现货指数比例时补一条参考估算, 主显示仍是实际求解值", async () => {
    mocks.fetchCoalIndexRatioSince.mockResolvedValue(1.152);
    mocks.getBackend.mockResolvedValue({
      solveJson: vi.fn().mockResolvedValue(JSON.stringify(makeResult(1_000, 3_700))),
      saveHistory: vi.fn(),
    });

    render(<TodayScreen onNavigate={vi.fn()} />);
    await screen.findByText("1000", { selector: ".cost-int" });

    // fob 900 * 1.152 + frt 100 = 1136.8 -> 1137; 运费不参与漂移
    const estimate = await screen.findByText(/随焦煤现货指数/);
    expect(estimate.textContent).toContain("+15.2%");
    expect(estimate.textContent).toContain("1137");
  });

  it("现货指数拿不到时只显示时效提示, 不编造估算", async () => {
    mocks.fetchCoalIndexRatioSince.mockResolvedValue(null);
    mocks.getBackend.mockResolvedValue({
      solveJson: vi.fn().mockResolvedValue(JSON.stringify(makeResult(1_000, 3_700))),
      saveHistory: vi.fn(),
    });

    render(<TodayScreen onNavigate={vi.fn()} />);
    await screen.findByText("1000", { selector: ".cost-int" });
    await screen.findByText(/报价停留在 2026-07-17/);

    expect(screen.queryByText(/随焦煤现货指数/)).toBeNull();
  });
});
