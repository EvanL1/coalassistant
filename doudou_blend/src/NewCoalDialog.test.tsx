// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NewCoalDialog } from "./NewCoalDialog";

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
      const selects = screen.getAllByRole("combobox");
      expect(selects.length).toBe(2);
      selects.forEach((s) => expect((s as HTMLSelectElement).disabled).toBe(true));
    });
  });
});
