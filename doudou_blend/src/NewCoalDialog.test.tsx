// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NewCoalDialog } from "./NewCoalDialog";
import { addUserCoal } from "./storage";

const mocks = vi.hoisted(() => ({ master: { value: null as unknown } }));

vi.mock("./master_loader", () => ({
  loadMaster: () => Promise.resolve(mocks.master.value),
}));
vi.mock("./storage", () => ({
  addUserCoal: vi.fn(),
  findDuplicateCoalName: () => null,
  normalizeCoalName: (s: string) => s.trim(),
}));

beforeEach(() => {
  mocks.master.value = {
    version: "1",
    updated_at: "2026-09-21",
    description: "",
    schema: {
      fields: {},
      coal_type: { 焦煤: "Vdaf 10~28, G>65", 肥煤: "G>85 且 Y>25" },
      form: { 精煤: "洗选后的低灰产品", 煤泥: "<0.5mm 细泥" },
      status: {},
      confidence_per_field: {},
    },
    default_contract: { name: "", specs: [] },
    coals: [],
  };
});

afterEach(cleanup);

describe("NewCoalDialog 的煤种/形态选项", () => {
  it("选项来自 master.schema, 不是前端写死的 —— 词表只有一份真源", async () => {
    render(<NewCoalDialog existing={[]} onClose={() => {}} />);
    await waitFor(() =>
      expect(screen.getByRole("option", { name: "焦煤" })).toBeTruthy(),
    );
    expect(screen.getByRole("option", { name: "肥煤" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "精煤" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "煤泥" })).toBeTruthy();
    // 「主焦煤」是贸易口语, 不在国标词表里; 自由文本时代它还是输入框的占位符.
    expect(screen.queryByRole("option", { name: "主焦煤" })).toBeNull();
  });

  it("schema 缺失时下拉禁用, 不退化成可以乱填", async () => {
    mocks.master.value = {
      version: "1",
      updated_at: "",
      description: "",
      schema: null,
      default_contract: { name: "", specs: [] },
      coals: [],
    };
    render(<NewCoalDialog existing={[]} onClose={() => {}} />);
    await waitFor(() => {
      // 产地的三个下拉另有名录, 不看 master.schema.
      const selects = screen
        .getAllByRole("combobox")
        .filter((s) => !s.getAttribute("aria-label")?.startsWith("产地"));
      expect(selects.length).toBe(2);
      selects.forEach((s) => expect((s as HTMLSelectElement).disabled).toBe(true));
    });
  });
});

describe("NewCoalDialog 的产地", () => {
  it("产地从省、市、县下拉里选, 存成三个字段, 不再有自由文本", async () => {
    render(<NewCoalDialog existing={[]} onClose={() => {}} />);
    const province = screen.getByRole("combobox", { name: "产地省份" }) as HTMLSelectElement;
    await waitFor(() => expect(province.disabled).toBe(false));
    fireEvent.change(screen.getByPlaceholderText("如: 老山兰"), { target: { value: "新煤" } });
    fireEvent.change(province, { target: { value: "山西省" } });
    fireEvent.change(screen.getByRole("combobox", { name: "产地城市" }), { target: { value: "晋中市" } });
    fireEvent.click(screen.getByRole("button", { name: /添加|保存|确定|新增/ }));
    expect(addUserCoal).toHaveBeenCalledTimes(1);
    const saved = vi.mocked(addUserCoal).mock.calls[0][0];
    expect(saved).toMatchObject({ name: "新煤", province: "山西省", city: "晋中市" });
    expect(saved).not.toHaveProperty("county");
    expect(saved).not.toHaveProperty("region");
  });
});
