import { describe, expect, it } from "vitest";
import { citiesOf, countiesOf, loadDivisions, placeFields, placeLabel, provincesOf } from "./place";

describe("产地字段", () => {
  it("逐级带: 上一级空着, 下一级不带; 没填的不出键", () => {
    expect(placeFields({ province: "山西省", city: "晋中市", county: "灵石县" }))
      .toEqual({ province: "山西省", city: "晋中市", county: "灵石县" });
    expect(placeFields({ province: "山西省", city: null, county: "灵石县" })).toEqual({ province: "山西省" });
    expect(placeFields({ province: "", city: "晋中市" })).toEqual({});
    expect(placeFields({})).toEqual({});
  });

  it("展示: 省市县连写; 没填省的旧数据退回自由文本; 都没有为空", () => {
    expect(placeLabel({ province: "山西省", city: "晋中市", county: "灵石县" })).toBe("山西省 晋中市 灵石县");
    expect(placeLabel({ province: "山西省" })).toBe("山西省");
    expect(placeLabel({ region: "山西灵石" })).toBe("山西灵石");
    // 新旧都有时以三个字段为准.
    expect(placeLabel({ province: "山西省", region: "山西灵石" })).toBe("山西省");
    expect(placeLabel({})).toBe("");
  });
});

describe("行政区划名录", () => {
  it("省 → 市 → 县逐级取; 上一级没选时下一级为空", async () => {
    const divisions = await loadDivisions();
    expect(provincesOf(divisions)).toContain("山西省");
    expect(citiesOf(divisions, "山西省")).toHaveLength(11);
    expect(countiesOf(divisions, "山西省", "晋中市")).toContain("灵石县");
    expect(citiesOf(divisions, null)).toEqual([]);
    expect(countiesOf(divisions, "山西省", null)).toEqual([]);
    // 县不属于这个市: 取不到.
    expect(countiesOf(divisions, "山西省", "临汾市")).not.toContain("灵石县");
    expect(countiesOf(divisions, "不存在的省", "晋中市")).toEqual([]);
  });

  it("名录只有名称, 三级齐全, 没有空名和重名", async () => {
    const divisions = await loadDivisions();
    expect(provincesOf(divisions).length).toBeGreaterThanOrEqual(31);
    for (const [province, cities] of Object.entries(divisions)) {
      const cityNames = Object.keys(cities);
      expect(cityNames.length, province).toBeGreaterThan(0);
      for (const [city, counties] of Object.entries(cities)) {
        expect(counties.length, `${province}/${city}`).toBeGreaterThan(0);
        expect(new Set(counties).size, `${province}/${city}`).toBe(counties.length);
        expect(counties.every((name) => typeof name === "string" && name.trim() === name && name.length > 0))
          .toBe(true);
      }
    }
  });
});
