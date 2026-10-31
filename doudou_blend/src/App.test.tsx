// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import App from "./App";

vi.mock("./auth", () => ({ getSession: async () => ({ authenticated: true, admin: false }) }));
vi.mock("./cloudStorage", () => ({ initializeCloudStorage: async () => () => undefined }));
vi.mock("./screens/TodayScreen", () => ({ TodayScreen: () => <div>今日内容</div> }));
vi.mock("./screens/CoalPoolScreen", () => ({ CoalPoolScreen: () => <div>煤池内容</div> }));
vi.mock("./screens/ContractScreen", () => ({ ContractScreen: () => <div>合同内容</div> }));
vi.mock("./mcci", async (original) => ({
  ...(await original<typeof import("./mcci")>()),
  fetchMcci: async () => [{
    code: "test", type_name: "肥煤", name: "肥煤指数", coal_type: "肥煤", props: { S: 1 },
    points: [{ date: "2026-10-30", price: 2200 }],
  }],
}));

afterEach(cleanup);

it("今日和煤池顶部显示行情条, 合同不显示", async () => {
  const { container } = render(<App />);
  await screen.findByText("肥煤");
  expect(container.querySelector(".screen")?.firstElementChild?.classList.contains("index-ticker")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: /煤池/ }));
  await screen.findByText("煤池内容");
  expect(container.querySelector(".index-ticker")).not.toBeNull();
  expect(container.querySelector(".screen")?.firstElementChild?.classList.contains("index-ticker")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: /合同/ }));
  expect(container.querySelector(".index-ticker")).toBeNull();
});
