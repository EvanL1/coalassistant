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
import type { HistoryRecord } from "../types";
import { HistoryScreen } from "./HistoryScreen";

const mocks = vi.hoisted(() => ({
  getBackend: vi.fn(),
  confirm: vi.fn(),
}));

vi.mock("../backend", () => ({
  getBackend: mocks.getBackend,
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const oldRecord: HistoryRecord = {
  id: "1",
  occurred_at: "2026-07-17T08:00:00.000Z",
  contract_name: "旧合同",
  cost_cif: 1_000,
  recipe: { 测试煤: 1 },
  mixed: null,
  csr_measured: null,
  s_measured: null,
  a_measured: null,
  v_measured: null,
  g_measured: null,
  y_measured: null,
  m_measured: null,
  cri_measured: null,
  m40_measured: null,
  m10_measured: null,
  bulk_density: null,
  coking_hours: null,
  flue_temp: null,
};

/** 有混合指标的记录才开放回填. */
const backfillRecord: HistoryRecord = {
  ...oldRecord,
  id: "2",
  contract_name: "可回填",
  mixed: { s: 0.8, a: 10, v: 24, g: 80, y: 16, m: 10 },
};

async function openBackfill(setMeasuredQuality = vi.fn(async () => {})) {
  mocks.getBackend.mockResolvedValue({
    listHistory: vi.fn(async () => [backfillRecord]),
    clearHistory: vi.fn(),
    setMeasuredQuality,
  });
  render(<HistoryScreen />);
  fireEvent.click(await screen.findByRole("button", { name: "+ 录入实测化验" }));
  return setMeasuredQuality;
}

beforeEach(() => {
  mocks.getBackend.mockReset();
  mocks.confirm.mockReset();
  mocks.confirm.mockReturnValue(true);
  vi.stubGlobal("confirm", mocks.confirm);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("HistoryScreen 刷新顺序", () => {
  it("清空后的新列表不会被更早的刷新结果覆盖", async () => {
    const staleRefresh = deferred<HistoryRecord[]>();
    const backend = {
      listHistory: vi
        .fn()
        .mockResolvedValueOnce([oldRecord])
        .mockImplementationOnce(() => staleRefresh.promise)
        .mockResolvedValueOnce([]),
      clearHistory: vi.fn(async () => {
        window.dispatchEvent(new CustomEvent("doudou:history_changed"));
      }),
      setMeasuredQuality: vi.fn(),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<HistoryScreen />);
    await screen.findByText("旧合同");

    act(() => {
      window.dispatchEvent(new CustomEvent("doudou:history_changed"));
    });
    await waitFor(() => expect(backend.listHistory).toHaveBeenCalledTimes(2));

    fireEvent.click(screen.getByRole("button", { name: "清空" }));
    await waitFor(() => expect(backend.listHistory).toHaveBeenCalledTimes(3));
    await screen.findByText("还没保存过方案");

    staleRefresh.resolve([oldRecord]);
    await act(async () => {
      await staleRefresh.promise;
    });

    expect(screen.queryByText("旧合同")).toBeNull();
  });
});

describe("HistoryScreen 回填焦炭与炼焦条件", () => {
  it("炼焦条件按各自量程保存, 不套 0~100", async () => {
    const save = await openBackfill();
    fireEvent.change(screen.getByLabelText("结焦时间"), { target: { value: "25" } });
    fireEvent.change(screen.getByLabelText("炉温"), { target: { value: "1350" } });
    fireEvent.change(screen.getByLabelText("焦CRI"), { target: { value: "24.5" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith("2", { coking_hours: 25, flue_temp: 1350, cri: 24.5 }),
    );
  });

  it("超出量程不保存并提示", async () => {
    const save = await openBackfill();
    fireEvent.change(screen.getByLabelText("装煤密度"), { target: { value: "2" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await screen.findByText("装煤密度 量程应在 0.5~1.5 t/m³");
    expect(save).not.toHaveBeenCalled();
  });
});
