// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MasterCoalEntry } from "./types";
import type { CoalPref } from "./storage";
import { CoalEditor } from "./CoalEditor";
import { setPenaltyTemplate } from "./penaltyStorage";

const mocks = vi.hoisted(() => ({
  getCoalPref: vi.fn(),
  setCoalPref: vi.fn(),
  clearCoalPref: vi.fn(),
  recordCoalQuote: vi.fn(),
  removeUserCoal: vi.fn(),
}));

vi.mock("./storage", () => ({
  getCoalPref: mocks.getCoalPref,
  setCoalPref: mocks.setCoalPref,
  clearCoalPref: mocks.clearCoalPref,
  recordCoalQuote: mocks.recordCoalQuote,
  removeUserCoal: mocks.removeUserCoal,
}));

const coal: MasterCoalEntry = {
  name: "测试主焦",
  status: "verified",
  props: { A: 9.5, G: 85 },
  fob: 1000,
  frt: 100,
};

/** 采购合同: 灰分保证值以内不扣, 每超 0.1% 扣 8 元/吨, 超 12% 拒收. */
const ashTemplate = {
  clauses: [
    {
      indicator: "A",
      direction: "Upper" as const,
      penalty: { tiers: [{ rate: 80 }], reject: 12 },
    },
  ],
};

afterEach(cleanup);

beforeEach(() => {
  localStorage.clear();
  mocks.getCoalPref.mockReset().mockReturnValue(null);
  mocks.setCoalPref.mockReset();
  mocks.clearCoalPref.mockReset();
  mocks.recordCoalQuote.mockReset();
  mocks.removeUserCoal.mockReset();
});

function savedPref(): Partial<CoalPref> {
  expect(mocks.setCoalPref).toHaveBeenCalledTimes(1);
  return mocks.setCoalPref.mock.calls[0][1] as Partial<CoalPref>;
}

describe("煤卡采购保证值", () => {
  it("填进去的保证值存到该煤的 purchase_guarantees 上", () => {
    setPenaltyTemplate(ashTemplate);
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("灰保证值"), { target: { value: "10" } });
    fireEvent.click(screen.getByText("保存"));
    expect(savedPref().purchase_guarantees).toEqual({ A: 10 });
  });

  it("清空保证值后该项不再留在 purchase_guarantees 里", () => {
    setPenaltyTemplate(ashTemplate);
    mocks.getCoalPref.mockReturnValue({ purchase_guarantees: { A: 10 } });
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    expect((screen.getByLabelText("灰保证值") as HTMLInputElement).value).toBe("10");
    fireEvent.change(screen.getByLabelText("灰保证值"), { target: { value: "" } });
    fireEvent.click(screen.getByText("保存"));
    expect(savedPref().purchase_guarantees).toBeUndefined();
  });

  it("保证值越过该条款的拒收线时当场报错 —— 不用等求解失败才知道", () => {
    setPenaltyTemplate(ashTemplate);
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("灰保证值"), { target: { value: "13" } });
    expect(screen.getByText(/拒收线不能低于保证值/)).toBeTruthy();
  });

  it("这项还没有采购扣款条款时只提示不拦 —— 用户可能先填保证值再配模板", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("灰保证值"), { target: { value: "10" } });
    expect(screen.getByText(/还没有灰的采购扣款条款/)).toBeTruthy();
    fireEvent.click(screen.getByText("保存"));
    expect(savedPref().purchase_guarantees).toEqual({ A: 10 });
  });

  it("重置为 master 默认值会连保证值一起清掉, 确认框里得说出来", () => {
    const confirmSpy = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirmSpy);
    mocks.getCoalPref.mockReturnValue({
      purchase_guarantees: { A: 10 },
      fob_override: 1200,
    });
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.click(screen.getByText("重置为 master 默认值"));
    expect(confirmSpy.mock.calls[0][0]).toContain("采购保证值");
    expect(mocks.clearCoalPref).not.toHaveBeenCalled();
  });

  it("没填保证值的煤不写出空的 purchase_guarantees", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.click(screen.getByText("保存"));
    expect(savedPref().purchase_guarantees).toBeUndefined();
  });
});
