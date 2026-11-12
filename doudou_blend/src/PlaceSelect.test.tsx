// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlaceSelect } from "./PlaceSelect";
import type { Place } from "./domain/place";

// 默认走真名录; 个别用例让加载失败.
const mocks = vi.hoisted(() => ({ fail: false }));
vi.mock("./domain/place", async (original) => {
  const real = await original<typeof import("./domain/place")>();
  return {
    ...real,
    loadDivisions: () => (mocks.fail ? Promise.reject(new Error("加载失败")) : real.loadDivisions()),
  };
});

afterEach(() => {
  cleanup();
  mocks.fail = false;
});

function Harness({ onChange, initial = {} }: { onChange: (place: Place) => void; initial?: Place }) {
  const [place, setPlace] = useState<Place>(initial);
  return (
    <PlaceSelect
      value={place}
      onChange={(next) => {
        setPlace(next);
        onChange(next);
      }}
    />
  );
}

const select = (name: string) => screen.getByRole("combobox", { name }) as HTMLSelectElement;
const optionsOf = (name: string) => Array.from(select(name).options).map((option) => option.value);

async function ready() {
  await waitFor(() => expect(select("产地省份").disabled).toBe(false));
}

describe("PlaceSelect", () => {
  it("逐级联动: 没选省时市、县不可选; 选了省才出本省的市, 选了市才出本市的县", async () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);
    await ready();
    expect(select("产地城市").disabled).toBe(true);
    expect(select("产地区县").disabled).toBe(true);

    fireEvent.change(select("产地省份"), { target: { value: "山西省" } });
    expect(onChange).toHaveBeenLastCalledWith({ province: "山西省", city: null, county: null });
    expect(optionsOf("产地城市")).toContain("晋中市");
    expect(optionsOf("产地城市")).not.toContain("榆林市");
    expect(select("产地区县").disabled).toBe(true);

    fireEvent.change(select("产地城市"), { target: { value: "晋中市" } });
    expect(onChange).toHaveBeenLastCalledWith({ province: "山西省", city: "晋中市", county: null });
    expect(optionsOf("产地区县")).toContain("灵石县");

    fireEvent.change(select("产地区县"), { target: { value: "灵石县" } });
    expect(onChange).toHaveBeenLastCalledWith({ province: "山西省", city: "晋中市", county: "灵石县" });
  });

  it("换了省, 市和县清空; 换了市, 县清空; 选回未选就是没填", async () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} initial={{ province: "山西省", city: "晋中市", county: "灵石县" }} />);
    await ready();
    expect(select("产地区县").value).toBe("灵石县");

    fireEvent.change(select("产地城市"), { target: { value: "临汾市" } });
    expect(onChange).toHaveBeenLastCalledWith({ province: "山西省", city: "临汾市", county: null });
    expect(select("产地区县").value).toBe("");

    fireEvent.change(select("产地省份"), { target: { value: "陕西省" } });
    expect(onChange).toHaveBeenLastCalledWith({ province: "陕西省", city: null, county: null });
    expect(select("产地城市").value).toBe("");
    expect(optionsOf("产地城市")).toContain("榆林市");

    fireEvent.change(select("产地省份"), { target: { value: "" } });
    expect(onChange).toHaveBeenLastCalledWith({ province: null, city: null, county: null });
    expect(select("产地城市").disabled).toBe(true);
  });

  it("名录加载失败: 直接写出来, 不留三个点不动的下拉框", async () => {
    mocks.fail = true;
    render(<Harness onChange={vi.fn()} />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("行政区划名录加载失败");
    expect(screen.queryByRole("combobox")).toBeNull();
  });
});
