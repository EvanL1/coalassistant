// @vitest-environment jsdom

import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { APP_EVENTS, emit, listen, useAppEvents, type AppEvent } from "./events";

afterEach(cleanup);

describe("应用内事件", () => {
  it("emit 只通知订阅了这个名字的, 退订后不再通知", () => {
    const prefs = vi.fn();
    const history = vi.fn();
    const stopPrefs = listen(["prefs_changed", "contract_changed"], prefs);
    const stopHistory = listen(["history_changed"], history);

    emit("prefs_changed");
    emit("contract_changed");
    expect(prefs).toHaveBeenCalledTimes(2);
    expect(history).not.toHaveBeenCalled();

    stopPrefs();
    emit("prefs_changed");
    emit("history_changed");
    expect(prefs).toHaveBeenCalledTimes(2);
    expect(history).toHaveBeenCalledTimes(1);
    stopHistory();
  });

  it("每个事件名都发得出去、收得到", () => {
    for (const name of APP_EVENTS) {
      const handler = vi.fn();
      const stop = listen([name], handler);
      emit(name);
      stop();
      expect(handler, name).toHaveBeenCalledTimes(1);
    }
  });

  it("useAppEvents 调的是最新一次渲染的 handler, 卸载后退订", () => {
    const seen: string[] = [];
    function Probe({ label, names }: { label: string; names: AppEvent[] }) {
      useAppEvents(names, () => seen.push(label));
      return null;
    }
    const view = render(<Probe label="第一次" names={["shovel_changed"]} />);
    emit("shovel_changed");
    view.rerender(<Probe label="第二次" names={["shovel_changed"]} />);
    emit("shovel_changed");
    emit("quantity_changed");
    view.rerender(<Probe label="换了事件" names={["quantity_changed"]} />);
    emit("shovel_changed");
    emit("quantity_changed");
    view.unmount();
    emit("quantity_changed");
    expect(seen).toEqual(["第一次", "第二次", "换了事件"]);
  });
});
