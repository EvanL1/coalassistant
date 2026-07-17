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
};

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
