// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App";

const sync = vi.hoisted(() => ({
  state: { status: "idle" as "idle" | "error", error: null as string | null },
  retry: vi.fn(),
}));

vi.mock("./auth", () => ({ getSession: async () => ({ authenticated: true, admin: false }) }));
vi.mock("./cloudStorage", () => ({
  initializeCloudStorage: async () => () => undefined,
  getSyncState: () => sync.state,
  retrySync: sync.retry,
  resetSyncState: () => undefined,
  resolveSyncConflict: async () => undefined,
  SYNC_STATUS_EVENT: "doudou:sync_status_changed",
}));
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

beforeEach(() => {
  sync.state = { status: "idle", error: null };
  sync.retry.mockReset();
});

afterEach(cleanup);

it("今日和煤池顶部显示行情条, 合同不显示", async () => {
  // 行情条与同步横幅装在页面最顶上的同一个吸顶容器里.
  const { container } = render(<App />);
  await screen.findByText("肥煤");
  expect(container.querySelector(".screen")?.firstElementChild?.querySelector(".index-ticker")).not.toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /煤池/ }));
  await screen.findByText("煤池内容");
  expect(container.querySelector(".index-ticker")).not.toBeNull();
  expect(container.querySelector(".screen")?.firstElementChild?.querySelector(".index-ticker")).not.toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /合同/ }));
  expect(container.querySelector(".index-ticker")).toBeNull();
});

it("任一接口回 401 时回到登录页并提示登录已过期", async () => {
  render(<App />);
  await screen.findByText("今日内容");
  act(() => {
    window.dispatchEvent(new CustomEvent("doudou:session_expired"));
  });
  expect(await screen.findByText("登录已过期, 请重新登录")).toBeTruthy();
  expect(screen.getByRole("button", { name: "登录" })).toBeTruthy();
  expect(screen.queryByText("今日内容")).toBeNull();
});

it("云端写回失败时常驻提示, 点重试再写一次", async () => {
  sync.state = { status: "error", error: "服务暂时不可用" };
  render(<App />);
  await screen.findByText("今日内容");
  expect(screen.getByText("有改动还没同步到云端")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(sync.retry).toHaveBeenCalledTimes(1);

  sync.state = { status: "idle", error: null };
  act(() => {
    window.dispatchEvent(new CustomEvent("doudou:sync_status_changed"));
  });
  expect(screen.queryByText("有改动还没同步到云端")).toBeNull();
});
