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
import { MeScreen } from "./MeScreen";

const mocks = vi.hoisted(() => ({
  getBackend: vi.fn(),
  getCoalPrefs: vi.fn(),
  getUserContract: vi.fn(),
  confirm: vi.fn(),
  shovelTons: { value: 3 },
  setShovelTons: vi.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

vi.mock("../backend", () => ({
  getBackend: mocks.getBackend,
}));

vi.mock("../auth", () => ({
  logout: vi.fn(),
}));

vi.mock("../storage", () => ({
  clearAllCoalPrefs: vi.fn(),
  clearUserContract: vi.fn(),
  getCoalPrefs: mocks.getCoalPrefs,
  getUserContract: mocks.getUserContract,
  getShovelTons: () => mocks.shovelTons.value,
  setShovelTons: mocks.setShovelTons,
}));

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

beforeEach(() => {
  mocks.getBackend.mockReset();
  mocks.getCoalPrefs.mockReset();
  mocks.getUserContract.mockReset();
  mocks.confirm.mockReset();
  mocks.setShovelTons.mockReset();
  mocks.shovelTons.value = 3;
  mocks.getCoalPrefs.mockReturnValue({});
  mocks.getUserContract.mockReturnValue(null);
  mocks.confirm.mockReturnValue(true);
  vi.stubGlobal("confirm", mocks.confirm);
});

describe("MeScreen 历史数据", () => {
  it("通过当前 Backend 读取并清空历史数量", async () => {
    const backend = {
      countHistory: vi.fn().mockResolvedValue(42),
      clearHistory: vi.fn().mockResolvedValue(undefined),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<MeScreen />);
    await screen.findByText("42 条");

    fireEvent.click(screen.getByRole("button", { name: /^清空历史/ }));

    await waitFor(() => expect(backend.clearHistory).toHaveBeenCalledTimes(1));
    await screen.findByText("0 条");
  });

  it("Backend 清空失败时保留原数量并显示错误", async () => {
    const backend = {
      countHistory: vi.fn().mockResolvedValue(5),
      clearHistory: vi.fn().mockRejectedValue(new Error("SQLite 错误")),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<MeScreen />);
    await screen.findByText("5 条");

    fireEvent.click(screen.getByRole("button", { name: /^清空历史/ }));

    await screen.findByText("清空历史失败，请重试");
    expect(screen.getByText("5 条")).toBeTruthy();
  });

  it("清空完成后不会被更早发出的计数请求覆盖", async () => {
    const staleCount = deferred<number>();
    const backend = {
      countHistory: vi.fn(() => staleCount.promise),
      clearHistory: vi.fn().mockResolvedValue(undefined),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<MeScreen />);
    await waitFor(() => expect(backend.countHistory).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: /^清空历史/ }));
    await screen.findByText("0 条");

    staleCount.resolve(9);
    await act(async () => {
      await staleCount.promise;
    });

    expect(screen.getByText("0 条")).toBeTruthy();
    expect(screen.queryByText("9 条")).toBeNull();
  });

  it("真实 Backend 清空事件触发的计数挂起时仍确定显示 0", async () => {
    const eventCount = deferred<number>();
    const backend = {
      countHistory: vi
        .fn()
        .mockResolvedValueOnce(4)
        .mockImplementationOnce(() => eventCount.promise),
      clearHistory: vi.fn(async () => {
        window.dispatchEvent(new CustomEvent("doudou:history_changed"));
      }),
    };
    mocks.getBackend.mockResolvedValue(backend);

    render(<MeScreen />);
    await screen.findByText("4 条");

    fireEvent.click(screen.getByRole("button", { name: /^清空历史/ }));
    await screen.findByText("0 条");

    eventCount.resolve(8);
    await act(async () => {
      await eventCount.promise;
    });

    expect(screen.getByText("0 条")).toBeTruthy();
    expect(screen.queryByText("8 条")).toBeNull();
  });
});

describe("MeScreen 一铲吨数", () => {
  beforeEach(() => {
    mocks.getBackend.mockResolvedValue({ countHistory: vi.fn().mockResolvedValue(0) });
  });

  it("改成合法值失焦后保存, 没改不重复保存", () => {
    render(<MeScreen />);
    const input = screen.getByLabelText("一铲吨数") as HTMLInputElement;
    expect(input.value).toBe("3");

    fireEvent.blur(input);
    expect(mocks.setShovelTons).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: "4.5" } });
    fireEvent.blur(input);
    expect(mocks.setShovelTons).toHaveBeenCalledWith(4.5);
  });

  it("非法值回退上次的值", () => {
    render(<MeScreen />);
    const input = screen.getByLabelText("一铲吨数") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "0" } });
    fireEvent.blur(input);
    expect(mocks.setShovelTons).not.toHaveBeenCalled();
    expect(input.value).toBe("3");
  });
});
