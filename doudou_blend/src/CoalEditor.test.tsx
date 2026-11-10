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

  it("重置前的确认框说明保证值会一并清掉, 点取消则什么都不动", () => {
    const confirmSpy = vi.fn().mockReturnValue(false);
    vi.stubGlobal("confirm", confirmSpy);
    mocks.getCoalPref.mockReturnValue({
      purchase_guarantees: { A: 10 },
      props_override: { S: 0.9 },
    });
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.click(screen.getByText("重置为煤库默认值"));
    expect(confirmSpy.mock.calls[0][0]).toContain("采购保证值");
    expect(mocks.clearCoalPref).not.toHaveBeenCalled();
    expect((screen.getByLabelText("灰保证值") as HTMLInputElement).value).toBe("10");
  });

  it("确认重置后保证值真的没了 —— 不是只在确认框里吓唬一下", () => {
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    mocks.getCoalPref.mockReturnValue({
      purchase_guarantees: { A: 10 },
      props_override: { S: 0.9 },
    });
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    expect((screen.getByLabelText("灰保证值") as HTMLInputElement).value).toBe("10");

    fireEvent.click(screen.getByText("重置为煤库默认值"));
    expect(mocks.clearCoalPref).toHaveBeenCalledWith(coal.name);
    // 界面上也跟着空了, 否则用户以为还在, 一保存又写回去
    expect((screen.getByLabelText("灰保证值") as HTMLInputElement).value).toBe("");

    fireEvent.click(screen.getByText("保存"));
    expect(savedPref().purchase_guarantees).toBeUndefined();
  });

  it("没填保证值的煤不写出空的 purchase_guarantees", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.click(screen.getByText("保存"));
    expect(savedPref().purchase_guarantees).toBeUndefined();
  });
});

describe("煤卡不录价", () => {
  it("没有出厂价/运费/价格锚点输入, 保存也不写价格字段", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    expect(screen.queryByText("出厂价 FOB")).toBeNull();
    expect(screen.queryByText("运费 FRT")).toBeNull();
    expect(screen.queryByText("作为价格锚点")).toBeNull();

    fireEvent.click(screen.getByText("保存"));
    const pref = savedPref() as Record<string, unknown>;
    for (const key of ["fob_override", "frt_override", "is_price_anchor", "fob_history"]) {
      expect(pref).not.toHaveProperty(key);
    }
  });
});

describe("煤卡数字输入", () => {
  it("化验值认不出时提示例子并挡住保存, 不按 parseFloat 截半截存", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    const ash = screen.getByLabelText("灰化验值") as HTMLInputElement;
    expect(ash.type).toBe("text");
    fireEvent.change(ash, { target: { value: "9.8x" } });
    expect(screen.getByText("请输入数字, 例如 65.3")).toBeTruthy();
    fireEvent.click(screen.getByText("保存"));
    expect(mocks.setCoalPref).not.toHaveBeenCalled();
    expect(screen.getByText(/灰化验值/, { selector: "[role=alert]" })).toBeTruthy();
  });

  it("全角数字和逗号小数点照常保存", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("灰化验值"), { target: { value: "９,８" } });
    fireEvent.click(screen.getByText("保存"));
    expect(savedPref().props_override).toEqual({ A: 9.8 });
  });

  it("敲到一半 (末尾逗号) 不闪红、底部不计数, 失焦才报; 中间夹空白立即报", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    const ash = screen.getByLabelText("灰化验值");
    fireEvent.change(ash, { target: { value: "1," } });
    expect(screen.queryByText("请输入数字, 例如 65.3")).toBeNull();
    expect(screen.queryByText(/处没填对/)).toBeNull();

    fireEvent.blur(ash);
    expect(screen.getByText("请输入数字, 例如 65.3")).toBeTruthy();
    expect(screen.getByText(/处没填对/)).toBeTruthy();

    // 中间夹空白不是中间态, 敲进去就报
    fireEvent.change(ash, { target: { value: "1 2" } });
    expect(screen.getByText("请输入数字, 例如 65.3")).toBeTruthy();
  });

  it("输到一半直接点保存: 拦住并把错亮出来", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("灰化验值"), { target: { value: "1," } });
    fireEvent.click(screen.getByText("保存"));
    expect(mocks.setCoalPref).not.toHaveBeenCalled();
    expect(screen.getByText("请输入数字, 例如 65.3")).toBeTruthy();
    expect(screen.getByText(/处没填对/)).toBeTruthy();
  });

  it("保证值认不出时同样挡住保存", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("灰保证值"), { target: { value: "1O" } });
    fireEvent.click(screen.getByText("保存"));
    expect(mocks.setCoalPref).not.toHaveBeenCalled();
  });
});

describe("煤卡停用煤", () => {
  it("煤矿已停用 (archived) 的煤启用开关点不动, 并写明原因", () => {
    render(<CoalEditor coal={{ ...coal, status: "archived" }} onClose={() => {}} />);
    const toggle = screen.getByRole("switch", { name: "在配煤中启用" });
    expect(toggle.getAttribute("aria-disabled")).toBe("true");
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText(/这种煤在煤库里已标为停用, 不能在配煤中启用/)).toBeTruthy();
  });

  it("正常煤的开关照常切换", () => {
    render(<CoalEditor coal={coal} onClose={() => {}} />);
    const toggle = screen.getByRole("switch", { name: "在配煤中启用" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-checked")).toBe("false");
  });
});
