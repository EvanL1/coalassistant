import { describe, expect, it } from "vitest";
import {
  isPartialNumberInput,
  numberInputError,
  numberOrNull,
  parseNumberInput,
  toHalfWidth,
} from "./numberInput";

describe("数字输入解析", () => {
  it("空和只有空白都算没填", () => {
    expect(parseNumberInput("", "price")).toEqual({ ok: true, value: null });
    expect(parseNumberInput("  ", "indicator")).toEqual({ ok: true, value: null });
    expect(parseNumberInput("\u3000", "indicator")).toEqual({ ok: true, value: null });
  });

  it("全角数字与小数点转半角", () => {
    expect(toHalfWidth("２０５０．５")).toBe("2050.5");
    expect(numberOrNull("２０５０", "price")).toBe(2050);
    expect(numberOrNull("６５．３", "indicator")).toBe(65.3);
    expect(numberOrNull(" 12 ", "indicator")).toBe(12);
  });

  it("价格认千分位, 不认别的逗号写法", () => {
    expect(numberOrNull("2,050", "price")).toBe(2050);
    expect(numberOrNull("1,234,567.5", "price")).toBe(1234567.5);
    expect(numberOrNull("２，０５０", "price")).toBe(2050);
    expect(numberInputError("2,05", "price")).toBe("请输入数字, 例如 2050");
    expect(numberInputError("20,50", "price")).toBe("请输入数字, 例如 2050");
  });

  it("指标把单个逗号当小数点, 多个逗号或混着小数点就报错", () => {
    expect(numberOrNull("65,3", "indicator")).toBe(65.3);
    expect(numberOrNull("6,5", "indicator")).toBe(6.5);
    expect(numberOrNull(",5", "indicator")).toBe(0.5);
    expect(numberInputError("1,2,3", "indicator")).toBe("请输入数字, 例如 65.3");
    expect(numberInputError("1.2,3", "indicator")).toBe("请输入数字, 例如 65.3");
  });

  it("认不出来就报错, 不吞字符", () => {
    for (const raw of ["abc", "12a", "1.2.3", "--1", "1e3", "Infinity", "."]) {
      expect(parseNumberInput(raw, "indicator").ok).toBe(false);
    }
    expect(numberOrNull("abc", "price")).toBeNull();
  });

  it("只去首尾空白: 数字中间夹空白是填错, 不是拼起来", () => {
    expect(numberOrNull("12 5", "indicator")).toBeNull();
    expect(numberInputError("12 5", "indicator")).toBe("请输入数字, 例如 65.3");
    expect(numberInputError("2 050", "price")).toBe("请输入数字, 例如 2050");
    expect(numberOrNull(" 12 ", "indicator")).toBe(12);
    expect(numberOrNull("\t65.3\n", "indicator")).toBe(65.3);
  });

  it("末尾是小数点/逗号/正负号、千分位写到一半, 算输到一半的中间态", () => {
    for (const raw of ["1.", "1,", "9,", "-", "+", ".", ",", "６５，", "2,0", "2,05", "1,2"]) {
      expect(isPartialNumberInput(raw)).toBe(true);
    }
    for (const raw of ["", "  ", "12 5", "2,050", "9.8x", "65", "-1.5", "2,0500"]) {
      expect(isPartialNumberInput(raw)).toBe(false);
    }
  });

  it("正负号与小数照常认", () => {
    expect(numberOrNull("-1.5", "indicator")).toBe(-1.5);
    expect(numberOrNull("+3", "price")).toBe(3);
    expect(numberOrNull(".5", "indicator")).toBe(0.5);
    expect(numberOrNull("5.", "indicator")).toBe(5);
    expect(numberInputError("80", "indicator")).toBeNull();
  });
});
