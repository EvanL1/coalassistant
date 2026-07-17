import { describe, expect, it } from "vitest";
import type { BlendRequest, BlendResult } from "../types";
import {
  isSnapshotActionable,
  LatestRequestTracker,
  type SolveSnapshot,
} from "./solveSession";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function makeSnapshot(
  requestId: number,
  quantity: number | null = 3_700,
  ok = true,
): SolveSnapshot {
  const request: BlendRequest = {
    coals: [],
    specs: [],
    total_quantity: quantity,
  };
  const result: BlendResult = {
    ok,
    recipe: {},
    orders: [],
    indicator_check: [],
    warnings: [],
  };
  return {
    requestId,
    request,
    result,
    contractName: "测试合同",
    enabledCount: 0,
  };
}

describe("LatestRequestTracker", () => {
  it("乱序完成时只接受最后签发的请求", async () => {
    const tracker = new LatestRequestTracker();
    const firstId = tracker.issue();
    const first = deferred<string>();
    const secondId = tracker.issue();
    const second = deferred<string>();
    const applied: string[] = [];

    const settle = async (requestId: number, result: Promise<string>) => {
      const value = await result;
      tracker.accept(requestId, () => applied.push(value));
    };
    const firstSettled = settle(firstId, first.promise);
    const secondSettled = settle(secondId, second.promise);

    second.resolve("新结果");
    await secondSettled;
    first.resolve("旧结果");
    await firstSettled;

    expect(applied).toEqual(["新结果"]);
    expect(tracker.currentRequestId).toBe(secondId);
  });

  it("失效后拒绝组件卸载前的在途响应", () => {
    const tracker = new LatestRequestTracker();
    const requestId = tracker.issue();

    tracker.invalidate();

    expect(tracker.isCurrent(requestId)).toBe(false);
    expect(tracker.accept(requestId, () => undefined)).toBe(false);
  });
});

describe("isSnapshotActionable", () => {
  it("仅允许当前请求、同一采购量且求解成功的快照", () => {
    const snapshot = makeSnapshot(3);

    expect(isSnapshotActionable(snapshot, 3, "3700")).toBe(true);
    expect(isSnapshotActionable(snapshot, 4, "3700")).toBe(false);
    expect(isSnapshotActionable(snapshot, 3, "5000")).toBe(false);
    expect(isSnapshotActionable(snapshot, 3, "")).toBe(false);
    expect(isSnapshotActionable(snapshot, 3, "-1")).toBe(false);
    expect(isSnapshotActionable(makeSnapshot(3, null), 3, "3700")).toBe(false);
    expect(isSnapshotActionable(makeSnapshot(3, 3_700, false), 3, "3700")).toBe(false);
    expect(isSnapshotActionable(null, 3, "3700")).toBe(false);
  });
});
