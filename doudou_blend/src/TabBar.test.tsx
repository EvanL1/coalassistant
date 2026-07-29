// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TabBar } from "./TabBar";

afterEach(cleanup);

describe("响应式主导航", () => {
  it("标记当前页面并保留桌面品牌信息", () => {
    render(<TabBar active="today" onChange={() => {}} />);

    expect(screen.getByRole("navigation", { name: "主导航" })).toBeTruthy();
    expect(screen.getByText("豆哥配煤")).toBeTruthy();
    expect(screen.getByText("智能配煤工作台")).toBeTruthy();
    expect(screen.getByRole("button", { name: "今日" }).getAttribute("aria-current"))
      .toBe("page");
  });

  it("点击桌面或移动导航都复用同一切屏行为", () => {
    const onChange = vi.fn();
    render(<TabBar active="today" onChange={onChange} />);

    fireEvent.click(screen.getByRole("button", { name: "煤池" }));

    expect(onChange).toHaveBeenCalledWith("pool");
  });
});
