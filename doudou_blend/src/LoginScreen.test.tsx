// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LoginRateLimitError } from "./auth";
import { LoginScreen } from "./LoginScreen";

const mocks = vi.hoisted(() => ({ tryLogin: vi.fn() }));

vi.mock("./auth", async (original) => ({
  ...(await original<typeof import("./auth")>()),
  tryLogin: mocks.tryLogin,
}));

beforeEach(() => {
  mocks.tryLogin.mockReset();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function submitForm(container: HTMLElement) {
  fireEvent.submit(container.querySelector("form")!);
}

it("密码错误提示账号或密码错误", async () => {
  mocks.tryLogin.mockResolvedValue(false);
  const { container } = render(<LoginScreen />);
  submitForm(container);
  expect(await screen.findByText(/账号或密码错误/)).toBeTruthy();
});

it("被限速 (429) 时提示多少秒后再试, 等待期间按钮不可点并倒计时", async () => {
  vi.useFakeTimers();
  mocks.tryLogin.mockRejectedValue(new LoginRateLimitError("尝试次数过多", 30));
  const { container } = render(<LoginScreen />);
  submitForm(container);
  await act(async () => undefined);

  expect(screen.getByText(/尝试次数过多, 请 30 秒后再试/)).toBeTruthy();
  const button = screen.getByRole("button", { name: "30 秒后再试" }) as HTMLButtonElement;
  expect(button.disabled).toBe(true);

  act(() => {
    vi.advanceTimersByTime(29_000);
  });
  expect(screen.getByRole("button", { name: "1 秒后再试" })).toBeTruthy();

  act(() => {
    vi.advanceTimersByTime(1_000);
  });
  expect((screen.getByRole("button", { name: "登录" }) as HTMLButtonElement).disabled).toBe(false);
});

it("限速但没给等待秒数时, 显示服务端的原因, 按钮不锁", async () => {
  mocks.tryLogin.mockRejectedValue(new LoginRateLimitError("尝试次数过多, 一分钟后再试", null));
  const { container } = render(<LoginScreen />);
  submitForm(container);

  expect(await screen.findByText(/尝试次数过多, 一分钟后再试/)).toBeTruthy();
  expect((screen.getByRole("button", { name: "登录" }) as HTMLButtonElement).disabled).toBe(false);
});

it("限速以外的登录失败显示服务端给的 reason", async () => {
  mocks.tryLogin.mockRejectedValue(new Error("登录服务维护中"));
  const { container } = render(<LoginScreen />);
  submitForm(container);
  expect(await screen.findByText(/登录服务维护中/)).toBeTruthy();
});

it("断网等网络层错误没有 reason, 才显示暂不可用", async () => {
  mocks.tryLogin.mockRejectedValue(new TypeError("Failed to fetch"));
  const { container } = render(<LoginScreen />);
  submitForm(container);
  expect(await screen.findByText(/登录服务暂不可用/)).toBeTruthy();
});
