// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CoalMaster, Spec } from "../types";
import { ContractScreen } from "./ContractScreen";

const mocks = vi.hoisted(() => ({
  loadMaster: vi.fn(),
  getUserContract: vi.fn(),
  setUserContract: vi.fn(),
  clearUserContract: vi.fn(),
}));

vi.mock("../master_loader", () => ({
  loadMaster: mocks.loadMaster,
}));

vi.mock("../storage", () => ({
  getUserContract: mocks.getUserContract,
  setUserContract: mocks.setUserContract,
  clearUserContract: mocks.clearUserContract,
}));

/** 只留两条: 一条上限型(可计价) + 一条区间型(core 不支持计价). */
const specs: Spec[] = [
  { indicator: "A", direction: "Upper", min: null, max: 10, enabled: true },
  { indicator: "petro", direction: "Range", min: 1.2, max: 1.4, enabled: true },
];

const master: CoalMaster = {
  version: "test",
  updated_at: "2026-09-19",
  description: "测试数据",
  default_contract: { name: "测试合同", specs },
  coals: [],
};

afterEach(cleanup);

beforeEach(() => {
  mocks.loadMaster.mockReset().mockResolvedValue(master);
  mocks.getUserContract.mockReset().mockReturnValue(null);
  mocks.setUserContract.mockReset();
  mocks.clearUserContract.mockReset();
});

async function renderScreen() {
  render(<ContractScreen />);
  await waitFor(() => expect(screen.getAllByLabelText(/按扣款计价/)).toHaveLength(2));
}

/** 上限型那条的计价开关(区间型那条在后面). */
function pricedToggle(index = 0): HTMLInputElement {
  return screen.getAllByLabelText(/按扣款计价/)[index] as HTMLInputElement;
}

function savedSpecs(): Spec[] {
  expect(mocks.setUserContract).toHaveBeenCalledTimes(1);
  return mocks.setUserContract.mock.calls[0][0] as Spec[];
}

describe("合同屏计价开关", () => {
  it("区间型指标的计价开关是关掉的 —— core 明确不支持区间型计价", async () => {
    await renderScreen();
    expect(pricedToggle(1).disabled).toBe(true);
    expect(pricedToggle(0).disabled).toBe(false);
  });

  it("没打开计价时, 档位表与拒收线一个都不出现", async () => {
    await renderScreen();
    expect(screen.queryByLabelText(/拒收线/)).toBeNull();
    expect(screen.queryByText(/加一档/)).toBeNull();
  });

  it("打开计价才出现档位表, 关掉又收回去", async () => {
    await renderScreen();
    fireEvent.click(pricedToggle());
    expect(screen.getByLabelText(/拒收线/)).toBeTruthy();
    fireEvent.click(pricedToggle());
    expect(screen.queryByLabelText(/拒收线/)).toBeNull();
  });

  it("关掉计价后保存: 约束级别回到硬约束, 不留扣款条款", async () => {
    await renderScreen();
    fireEvent.click(pricedToggle());
    fireEvent.click(pricedToggle());
    fireEvent.click(screen.getByText("保存合同"));
    expect(savedSpecs()[0].enforcement).toBe("Hard");
    expect(savedSpecs()[0].penalty).toBeNull();
  });

  it("软约束的指标打开计价再关掉, 回到软约束而不是被悄悄收紧成硬约束", async () => {
    mocks.getUserContract.mockReturnValue([
      { ...specs[0], enforcement: "Soft" },
      specs[1],
    ]);
    await renderScreen();
    fireEvent.click(pricedToggle());
    fireEvent.click(pricedToggle());
    fireEvent.click(screen.getByText("保存合同"));
    expect(savedSpecs()[0].enforcement).toBe("Soft");
  });
});

describe("合同屏档位录入", () => {
  /** 照抄合同"灰分每超 0.1% 扣 8 元/吨, 超 12% 拒收". */
  function fillAshClause({ amount = "8", reject = "12" } = {}) {
    fireEvent.click(pricedToggle());
    fireEvent.change(screen.getByLabelText(/每/), { target: { value: "0.1" } });
    fireEvent.change(screen.getByLabelText(/档扣款/), { target: { value: amount } });
    fireEvent.change(screen.getByLabelText(/拒收线/), { target: { value: reject } });
  }

  it("照抄的每 0.1% 扣 8 元, 存下去是 80 元/吨·% —— 差一个量级也不会有东西看起来坏掉", async () => {
    await renderScreen();
    fillAshClause();
    fireEvent.click(screen.getByText("保存合同"));
    const saved = savedSpecs()[0];
    expect(saved.enforcement).toBe("Priced");
    expect(saved.penalty).toEqual({ tiers: [{ rate: 80 }], reject: 12 });
  });

  it("扣款金额留空时报错并挡住保存, 不会静默存成 0 元/吨", async () => {
    await renderScreen();
    fillAshClause({ amount: "" });
    expect(screen.getByText(/「扣」要填/)).toBeTruthy();
    fireEvent.click(screen.getByText("保存合同"));
    expect(mocks.setUserContract).not.toHaveBeenCalled();
  });

  it("拒收线低于合同上限时报错并挡住保存", async () => {
    await renderScreen();
    fillAshClause({ reject: "9" });
    expect(screen.getByText(/拒收线不能低于合同上限/)).toBeTruthy();
    fireEvent.click(screen.getByText("保存合同"));
    expect(mocks.setUserContract).not.toHaveBeenCalled();
  });

  it("后一档扣得比前一档松时报错并挡住保存", async () => {
    await renderScreen();
    fillAshClause();
    fireEvent.click(screen.getByText(/加一档/));
    const amounts = screen.getAllByLabelText(/档扣款/);
    fireEvent.change(amounts[1], { target: { value: "6" } });
    fireEvent.change(screen.getByLabelText(/本档覆盖/), { target: { value: "0.5" } });
    expect(screen.getByText(/第 2 档要比上一档扣得更狠/)).toBeTruthy();
    fireEvent.click(screen.getByText("保存合同"));
    expect(mocks.setUserContract).not.toHaveBeenCalled();
  });

  it("两档都填对时存下两档, 非末档带覆盖宽度、末档不带", async () => {
    await renderScreen();
    fillAshClause();
    fireEvent.click(screen.getByText(/加一档/));
    const amounts = screen.getAllByLabelText(/档扣款/);
    fireEvent.change(amounts[1], { target: { value: "15" } });
    fireEvent.change(screen.getByLabelText(/本档覆盖/), { target: { value: "0.5" } });
    fireEvent.click(screen.getByText("保存合同"));
    expect(savedSpecs()[0].penalty).toEqual({
      tiers: [{ rate: 80, width: 0.5 }, { rate: 150 }],
      reject: 12,
    });
  });

  it("已存的计价合同重新进屏时回显出来, 不是空白", async () => {
    mocks.getUserContract.mockReturnValue([
      {
        ...specs[0],
        enforcement: "Priced",
        penalty: { tiers: [{ rate: 80 }], reject: 12 },
      },
      specs[1],
    ]);
    await renderScreen();
    expect(pricedToggle().checked).toBe(true);
    expect((screen.getByLabelText(/拒收线/) as HTMLInputElement).value).toBe("12");
    expect((screen.getByLabelText(/档扣款/) as HTMLInputElement).value).toBe("80");
  });
});
