import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ getMasterJson: vi.fn() }));
vi.mock("./backend", () => ({
  getBackend: async () => ({ getMasterJson: mocks.getMasterJson }),
}));

import { invalidateMaster, loadMaster } from "./master_loader";

beforeEach(() => {
  invalidateMaster();
  mocks.getMasterJson.mockReset();
});

describe("master 缓存", () => {
  it("并发调用只发一次请求", async () => {
    mocks.getMasterJson.mockResolvedValue('{"version":"v1"}');
    const [a, b] = await Promise.all([loadMaster(), loadMaster()]);
    expect(a).toBe(b);
    expect(a.version).toBe("v1");
    await loadMaster();
    expect(mocks.getMasterJson).toHaveBeenCalledTimes(1);
  });

  it("失败后不缓存, 下次重新请求", async () => {
    mocks.getMasterJson
      .mockRejectedValueOnce(new Error("断网"))
      .mockResolvedValueOnce('{"version":"v2"}');
    await expect(loadMaster()).rejects.toThrow("断网");
    await expect(loadMaster()).resolves.toMatchObject({ version: "v2" });
    expect(mocks.getMasterJson).toHaveBeenCalledTimes(2);
  });

  it("invalidate 后重新请求", async () => {
    mocks.getMasterJson.mockResolvedValue('{"version":"v3"}');
    await loadMaster();
    invalidateMaster();
    await loadMaster();
    expect(mocks.getMasterJson).toHaveBeenCalledTimes(2);
  });
});
