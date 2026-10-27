// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { PriceRobustnessCard } from "./PriceRobustnessCard";
import { coalTypeMap } from "../domain/coalTypes";
import type { BlendRequest, PriceSwing, RobustnessReport } from "../types";

function swing(name: string, extra: Partial<PriceSwing>): PriceSwing {
  return {
    name,
    share: 0.2,
    estimated: true,
    uncertainty: 250,
    rise: null,
    fall: null,
    ...extra,
  };
}

function report(extra: Partial<RobustnessReport> = {}): RobustnessReport {
  return {
    tolerance: 10,
    swing_range: 300,
    price_error: 214.4,
    swings: [
      swing("明鑫", { share: 0.37, rise: 25, fall: 150, coal_type: "瘦煤", ask: true }),
      swing("东泰鑫源", { share: 0, fall: 5, coal_type: "肥煤", ask: true }),
      swing("零头", { share: 0.003, rise: 5 }),
      swing("豹子沟", { share: 0.19, uncertainty: 60, rise: 90, coal_type: "焦煤" }),
    ],
    robust: {
      recipe: { 桃园: 0.6, 明鑫: 0.4 },
      cost: 1615.7,
      extra_cost: 9.9,
      price_error: 138.2,
    },
    ...extra,
  };
}

afterEach(() => {
  cleanup();
});

describe("PriceRobustnessCard", () => {
  it("没有分析结果时不渲染", () => {
    const { container } = render(
      <PriceRobustnessCard report={undefined} types={new Map()} />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("列出门槛与先问价标记, 并给出更稳的配方", () => {
    render(
      <PriceRobustnessCard
        report={report()}
        types={new Map([["桃园", "焦煤"]])}
      />,
    );
    // 先问价只认核心给的标记: 零头门槛也小但没标, 不列.
    expect(screen.getByText(/先问 明鑫、东泰鑫源 的价/)).toBeTruthy();
    expect(screen.getAllByText("先问价")).toHaveLength(2);
    const row = screen.getByText("东泰鑫源").closest("tr")!;
    expect(row.textContent).toContain("未入选");
    expect(row.textContent).toContain("≤5");
    expect(screen.getByText("零头").closest("tr")!.textContent).toContain("<0.5%");
    expect(screen.getByText(/平均估价误差 ±214 元\/吨 —— 不是成本误差/)).toBeTruthy();
    expect(screen.getByText(/按当前估算价多花 9.9 元\/吨/)).toBeTruthy();
  });

  it("没有更稳的配方时说明当前已最稳", () => {
    render(
      <PriceRobustnessCard
        report={report({ robust: null, swings: [] })}
        types={new Map()}
      />,
    );
    expect(screen.getByText(/没找到明显更稳的配方/)).toBeTruthy();
    expect(screen.getByText(/不足以改变配方/)).toBeTruthy();
  });
});

describe("coalTypeMap", () => {
  it("进 LP 的煤与按估算价求解的煤都取煤种", () => {
    const request: BlendRequest = {
      coals: [{ name: "甲", props: {}, fob: 1, frt: 0, coal_type: "焦煤" }],
      specs: [],
      price_factor: {
        adjusts: [],
        extra_coals: [
          { name: "乙", props: {}, fob: 0, frt: 0, coal_type: "瘦煤" },
          { name: "丙", props: {}, fob: 0, frt: 0 },
        ],
      },
    };
    const types = coalTypeMap(request);
    expect(types.get("甲")).toBe("焦煤");
    expect(types.get("乙")).toBe("瘦煤");
    expect(types.has("丙")).toBe(false);
  });

  it("形态与煤种并列: 精煤不标, 筛精/浮精用简称, 词表外照原样", () => {
    const request: BlendRequest = {
      coals: [
        { name: "洗", props: {}, fob: 1, frt: 0, coal_type: "焦煤", form: "精煤" },
        { name: "筛", props: {}, fob: 1, frt: 0, coal_type: "肥煤", form: "筛选精煤" },
        { name: "浮", props: {}, fob: 1, frt: 0, coal_type: "焦煤", form: "浮选精煤" },
        { name: "泥", props: {}, fob: 1, frt: 0, form: "煤泥" },
      ],
      specs: [],
    };
    const types = coalTypeMap(request);
    expect(types.get("洗")).toBe("焦煤");
    expect(types.get("筛")).toBe("肥煤·筛精");
    expect(types.get("浮")).toBe("焦煤·浮精");
    expect(types.get("泥")).toBe("煤泥");
  });
});
