// @vitest-environment jsdom
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { BlendResult, IndicatorCheck } from "../types";
import { IndicatorCard } from "./BlendCards";

afterEach(cleanup);

function check(over: Partial<IndicatorCheck>): IndicatorCheck {
  return {
    indicator: "S",
    label_zh: "硫",
    value: 1,
    min: null,
    max: 1,
    slack: 0,
    binding: false,
    proxy_value: null,
    evaluated_value: null,
    judged_value: null,
    uncertainty: null,
    method: "Linear",
    status: "Pass",
    model: null,
    penalty_per_ton: null,
    ...over,
  } as IndicatorCheck;
}

function resultWith(checks: IndicatorCheck[], over: Partial<BlendResult> = {}): BlendResult {
  return {
    ok: true,
    recipe: {},
    orders: [],
    indicator_check: checks,
    warnings: [],
    ...over,
  } as BlendResult;
}

function cellOf(label: string): HTMLElement {
  const cell = screen.getByText(label).closest(".indicator-cell");
  if (!(cell instanceof HTMLElement)) throw new Error(`找不到 ${label} 的指标格`);
  return cell;
}

describe("混合指标卡", () => {
  it("大字是真实值, 判定值另起一行; 靠判定规则才算过的标「按规则达标」", () => {
    render(
      <IndicatorCard
        result={resultWith([
          check({ value: 1.09989, evaluated_value: 1.09989, judged_value: 1, status: "TolerancePass" }),
        ])}
      />,
    );
    const cell = within(cellOf("硫"));
    expect(cell.getByText("1.0998")).toBeTruthy();
    expect(cell.getByText("按合同规则记 1")).toBeTruthy();
    expect(cell.getByText("按规则达标")).toBeTruthy();
    expect(cell.queryByText(/原始/)).toBeNull();
  });

  it("真实值截掉多余小数而不是四舍五入, 不会显示成越过判定格的数", () => {
    render(
      <IndicatorCard
        result={resultWith([
          check({ value: 1.099995, evaluated_value: 1.099995, judged_value: 1, status: "TolerancePass" }),
          check({
            indicator: "G",
            label_zh: "粘结",
            value: 84.99996,
            evaluated_value: 84.99996,
            judged_value: 84.9,
            min: 85,
            max: null,
            status: "Fail",
          }),
          check({
            indicator: "A",
            label_zh: "灰",
            value: 0.57,
            evaluated_value: 0.57,
            judged_value: 0.5,
            max: 10,
          }),
        ])}
      />,
    );
    // 四舍五入会成 1.1, 旁边却写「记 1」.
    expect(within(cellOf("硫")).getByText("1.0999")).toBeTruthy();
    // 四舍五入会成 85, 旁边却写「记 84.9」且判未通过.
    expect(within(cellOf("粘结")).getByText("84.9999")).toBeTruthy();
    // 0.57 × 10000 在二进制里是 5699.999…, 不能截成 0.5699.
    expect(within(cellOf("灰")).getByText("0.57")).toBeTruthy();
  });

  it("顶在下限上、只比下限低一丝求解残差的值, 按达标的数显示, 不另起一行", () => {
    render(
      <IndicatorCard
        result={resultWith([
          check({
            indicator: "Y",
            label_zh: "胶质",
            value: 20.7999999968,
            evaluated_value: 20.7999999968,
            judged_value: 20.8,
            min: 20.8,
            max: null,
          }),
          // 按原值判定的项: 判定值 = 原值减一点放行量, 不是另一个数.
          check({
            indicator: "petro",
            label_zh: "岩相",
            value: 0.1234713,
            evaluated_value: 0.1234713,
            judged_value: 0.1234712,
            max: 0.15,
          }),
        ])}
      />,
    );
    const y = within(cellOf("胶质"));
    expect(y.getByText("20.8")).toBeTruthy();
    expect(y.queryByText(/按合同规则记/)).toBeNull();
    const petro = within(cellOf("岩相"));
    expect(petro.getByText("0.1235")).toBeTruthy();
    expect(petro.queryByText(/按合同规则记/)).toBeNull();
  });

  it("四舍五入判定时, 真实值比判定值小也往判定值那一侧取", () => {
    render(
      <IndicatorCard
        result={resultWith([
          check({ value: 0.96004, evaluated_value: 0.96004, judged_value: 1, status: "Pass" }),
        ])}
      />,
    );
    const cell = within(cellOf("硫"));
    expect(cell.getByText("0.9601")).toBeTruthy();
    expect(cell.getByText("按合同规则记 1")).toBeTruthy();
  });

  it("真实值与判定值相同时不重复写", () => {
    render(
      <IndicatorCard
        result={resultWith([check({ value: 0.8, evaluated_value: 0.8, judged_value: 0.8 })])}
      />,
    );
    const cell = within(cellOf("硫"));
    expect(cell.getByText("0.8")).toBeTruthy();
    expect(cell.queryByText(/按合同规则记/)).toBeNull();
  });

  it("捣固估算标明只是参考; 低于合同下限时点出来", () => {
    const csr = check({
      indicator: "CSR",
      label_zh: "焦炭强度",
      value: 68,
      evaluated_value: 68,
      judged_value: 68,
      min: 68,
      max: null,
      status: "Unverified",
    });
    const { unmount } = render(
      <IndicatorCard result={resultWith([csr], { csr_stamp_estimate: 62.34 })} />,
    );
    expect(
      screen.getByText("捣固公式估算 62.3 · 仅供参考, 不参与达标判定 · 低于合同下限 68"),
    ).toBeTruthy();
    unmount();

    render(<IndicatorCard result={resultWith([csr], { csr_stamp_estimate: 70.12 })} />);
    expect(screen.getByText("捣固公式估算 70.1 · 仅供参考, 不参与达标判定")).toBeTruthy();
  });
});

describe("配方卡", () => {
  const order = (coal: string, ratio: number, shovels: number) => ({
    coal,
    ratio,
    tons: ratio * 300,
    shovels,
    fob_amount: 0,
    frt_amount: 0,
    cif_amount: 0,
    cif_eff_per_ton: 0,
    cif_eff_amount: 0,
  });

  it("为凑成整铲留余量重配时, 说明比不凑整贵多少", async () => {
    const { RecipeCard } = await import("./BlendCards");
    const result = resultWith([], {
      recipe: { 甲: 0.99, 乙: 0.01 },
      orders: [order("甲", 0.99, 99), order("乙", 0.01, 1)],
      shovel_extra_cost: 2.196,
    } as Partial<BlendResult>);
    render(<RecipeCard result={result} coalTypes={new Map()} shovelTons={3} />);
    expect(screen.getByText("为凑成整铲留了余量重新配, 比不按整铲配贵 2.20 元/吨")).toBeTruthy();
    expect(screen.queryByText(/没凑成整铲/)).toBeNull();
  });

  it("没走留余量重配时不显示这句", async () => {
    const { RecipeCard } = await import("./BlendCards");
    const result = resultWith([], {
      recipe: { 甲: 1 },
      orders: [order("甲", 1, 100)],
    } as Partial<BlendResult>);
    render(<RecipeCard result={result} coalTypes={new Map()} shovelTons={3} />);
    expect(screen.queryByText(/为凑成整铲/)).toBeNull();
  });
});
