// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BenchmarkCard, draftToBenchmark } from "./BenchmarkCard";

const mocks = vi.hoisted(() => ({ fetchBenchmarks: vi.fn() }));

vi.mock("../benchmarks", () => ({
  fetchBenchmarks: mocks.fetchBenchmarks,
  createBenchmark: vi.fn(),
  deleteBenchmark: vi.fn(),
}));

afterEach(() => {
  cleanup();
});

function draft(extra: Partial<Parameters<typeof draftToBenchmark>[0]> = {}) {
  return {
    name: "晋中中硫主焦",
    coal_type: "焦煤",
    form: "精煤",
    region: " 山西晋中 ",
    price: "2050",
    date: "2026-10-16",
    source: "",
    props: { S: "1.3", A: "10.5", V: "", G: "80", Y: "" },
    ...extra,
  };
}

describe("draftToBenchmark", () => {
  it("空指标不带, 数字转好", () => {
    expect(draftToBenchmark(draft())).toEqual({
      name: "晋中中硫主焦",
      coal_type: "焦煤",
      form: "精煤",
      region: "山西晋中",
      props: { S: 1.3, A: 10.5, G: 80 },
      price: 2050,
      date: "2026-10-16",
      source: undefined,
    });
    expect(draftToBenchmark(draft({ region: "" })))
      .toMatchObject({ region: undefined });
  });

  it("缺品名、价格非正、缺硫分都拦下", () => {
    expect(draftToBenchmark(draft({ name: " " }))).toBe("请填品名");
    expect(draftToBenchmark(draft({ price: "0" }))).toBe("报价须为正数");
    expect(
      draftToBenchmark(draft({ props: { S: "", A: "", V: "", G: "", Y: "" } })),
    ).toBe("至少要填硫分 S");
  });
});

describe("BenchmarkCard", () => {
  it("列出基准, 推算到今天的价与出处一并显示, 只有补录的能删", async () => {
    mocks.fetchBenchmarks.mockResolvedValue([
      {
        name: "蒲县单一肥煤",
        coal_type: "肥煤",
        form: "筛选精煤",
        region: "山西临汾",
        props: { S: 3.8, A: 11, G: 95, Y: 32 },
        price: 2145,
        date: "2026-10-09",
        current_price: 2101,
        origin: "master",
      },
      {
        name: "晋中中硫主焦",
        coal_type: "焦煤",
        props: { S: 1.3, A: 10.5, G: 80 },
        price: 2050,
        date: "2026-10-16",
        origin: "stored",
        id: "abc",
      },
    ]);
    render(<BenchmarkCard />);
    expect(await screen.findByText(/蒲县单一肥煤/)).toBeTruthy();
    expect(screen.getByText("S3.8 A11 G95 Y32")).toBeTruthy();
    // 煤种、形态、地区并列显示.
    expect(screen.getByText("肥煤·筛精 · 山西临汾")).toBeTruthy();
    expect(screen.getByText("今 ≈ 2101")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "删除" })).toHaveLength(1);
  });
});
