import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  getSyncState,
  hasUnsyncedChanges,
  initializeCloudStorage,
  resetSyncState,
  resolveSyncConflict,
  retrySync,
  SYNC_STATUS_EVENT,
} from "./cloudStorage";
import {
  canonicalUserSnapshot,
  getCoalPrefs,
  getShovelTons,
  getUserStorageSnapshot,
  setCoalPref,
  setShovelTons,
  type UserStorageSnapshot,
} from "./storage";

const KEY_SYNC_PENDING = "doudou_blend.sync_pending.v1";
const KEY_SYNC_BASELINE = "doudou_blend.sync_baseline.v1";

/** 与服务端返回形状一致的快照; 服务端老记录不带 shovel_tons, 解析后补默认值 3. */
function snapshotOf(over: Partial<UserStorageSnapshot>): UserStorageSnapshot {
  return {
    coal_prefs: {},
    contract: null,
    quantity: 3700,
    user_coals: [],
    shovel_tons: 3,
    ...over,
  };
}

/** 基线 = 上次确认与云端一致时的快照; 测试里直接种下, 模拟"上一次同步成功". */
function seedBaseline(snapshot: UserStorageSnapshot): void {
  localStorage.setItem(KEY_SYNC_BASELINE, canonicalUserSnapshot(snapshot));
}

/**
 * 假云端: 读返回当前内容, 写成功就更新; 可以指定下一次读或写失败, 或让写悬着等放行.
 * 写回路径每笔都是"先读再写", 按调用次序排假响应太脆, 所以按方法路由.
 */
function fakeCloud(initial: UserStorageSnapshot) {
  let cloud = initial;
  const puts: { body: UserStorageSnapshot; keepalive: boolean }[] = [];
  const failures: { method: string; reason: string }[] = [];
  let gets = 0;
  let holdPut: Promise<void> | null = null;
  vi.mocked(fetch).mockImplementation(async (input, init) => {
    const method = init?.method ?? "GET";
    if (String(input) !== "/api/storage") return Response.json({ ok: true });
    const failing = failures.findIndex((failure) => failure.method === method);
    if (failing >= 0) {
      const [failure] = failures.splice(failing, 1);
      return new Response(JSON.stringify({ ok: false, reason: failure.reason }), { status: 503 });
    }
    if (method === "GET") {
      gets += 1;
      return Response.json({ initialized: true, ...cloud });
    }
    const body = JSON.parse(init?.body as string) as UserStorageSnapshot;
    puts.push({ body, keepalive: init?.keepalive === true });
    if (holdPut) await holdPut;
    cloud = body;
    return Response.json({ initialized: true });
  });
  return {
    puts,
    get gets() {
      return gets;
    },
    get content() {
      return cloud;
    },
    /** 别的设备改了云端. */
    changedElsewhere(next: UserStorageSnapshot) {
      cloud = next;
    },
    failNext(method: "GET" | "PUT", reason: string) {
      failures.push({ method, reason });
    },
    /** 之后的写悬着, 调返回的函数才放行. */
    holdWrites(): () => void {
      let release!: () => void;
      holdPut = new Promise<void>((done) => {
        release = done;
      });
      return () => {
        holdPut = null;
        release();
      };
    },
  };
}

function makeStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

beforeEach(() => {
  vi.stubGlobal("window", new EventTarget());
  vi.stubGlobal("localStorage", makeStorage());
  vi.stubGlobal("fetch", vi.fn());
  if (typeof CustomEvent === "undefined") {
    vi.stubGlobal(
      "CustomEvent",
      class<T> extends Event {
        detail: T;

        constructor(type: string, init?: CustomEventInit<T>) {
          super(type);
          this.detail = init?.detail as T;
        }
      },
    );
  }
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("Web PostgreSQL 状态同步", () => {
  it("已有云端状态会覆盖本地缓存并持续写回", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(
        Response.json({
          initialized: true,
          coal_prefs: { 云端煤: { enabled: true } },
          contract: null,
          quantity: 4200,
          user_coals: [],
        }),
      )
      .mockResolvedValue(Response.json({ initialized: true }));

    const cleanup = await initializeCloudStorage();

    expect(getCoalPrefs()).toEqual({ 云端煤: { enabled: true } });
    expect(localStorage.getItem("doudou_blend.quantity.v1")).toBe("4200");
    // 一铲吨数上线前的服务端不回这个字段: 按默认值.
    expect(localStorage.getItem("doudou_blend.shovel_tons.v1")).toBe("3");

    setCoalPref("云端煤", { enabled: false });
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(fetchMock.mock.calls[1]?.[0]).toBe("/api/storage");
    cleanup();
  });

  it("空数据库首次启动会上传本地数据并导入历史", async () => {
    localStorage.setItem(
      "doudou_blend.coal_prefs.v1",
      JSON.stringify({ 本地煤: { enabled: true } }),
    );
    localStorage.setItem(
      "doudou_blend.history.v1",
      JSON.stringify([
        {
          id: "legacy-1",
          occurred_at: "2026-07-29T00:00:00.000Z",
          cost_cif: 1000,
          recipe: { 本地煤: 1 },
          contract_name: "默认合同",
        },
      ]),
    );
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(
        Response.json({
          initialized: false,
          coal_prefs: {},
          contract: null,
          quantity: 3700,
          user_coals: [],
        }),
      )
      .mockResolvedValueOnce(Response.json({ initialized: true }))
      .mockResolvedValueOnce(Response.json({ imported: 1 }));

    const cleanup = await initializeCloudStorage();

    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/api/storage",
      "/api/storage",
      "/api/history/import",
    ]);
    expect(localStorage.getItem("doudou_blend.history.v1")).toBeNull();
    cleanup();
  });

  it("一铲吨数随云端同步; 改了会写回, 非法值拒收", async () => {
    const fetchMock = vi.mocked(fetch);
    const cloud = fakeCloud(snapshotOf({ shovel_tons: 5 }));

    const cleanup = await initializeCloudStorage();
    expect(getShovelTons()).toBe(5);

    setShovelTons(2.5);
    await vi.waitFor(() => expect(cloud.puts).toHaveLength(1));
    expect(cloud.puts[0]?.body.shovel_tons).toBe(2.5);
    cleanup();

    fetchMock.mockReset();
    fetchMock.mockResolvedValueOnce(
      Response.json({
        initialized: true,
        coal_prefs: {},
        contract: null,
        quantity: 3700,
        user_coals: [],
        shovel_tons: 0,
      }),
    );
    await expect(initializeCloudStorage()).rejects.toThrow("字段无效");
  });
  it("写回失败时记下未同步并报错, 重试成功后清掉", async () => {
    const fetchMock = vi.mocked(fetch);
    const cloud = fakeCloud(snapshotOf({}));
    let changes = 0;
    window.addEventListener(SYNC_STATUS_EVENT, () => {
      changes += 1;
    });

    const cleanup = await initializeCloudStorage();
    cloud.failNext("PUT", "数据库忙");
    setCoalPref("甲", { enabled: true });
    await vi.waitFor(() => expect(getSyncState()).toEqual({ status: "error", error: "数据库忙" }));
    expect(hasUnsyncedChanges()).toBe(true);
    expect(changes).toBeGreaterThan(0);

    retrySync();
    await vi.waitFor(() => expect(getSyncState().status).toBe("idle"));
    expect(hasUnsyncedChanges()).toBe(false);
    expect(cloud.content.coal_prefs.甲?.enabled).toBe(true);
    cleanup();
    const calls = fetchMock.mock.calls.length;
    retrySync();
    expect(fetchMock).toHaveBeenCalledTimes(calls);
  });

  it("页面开着期间云端被别人改过: 写回前发现, 不推, 进入冲突", async () => {
    const cloud = fakeCloud(snapshotOf({}));
    const cleanup = await initializeCloudStorage();

    // 别的设备存了合同吨数与一种煤; 这台设备没刷新, 接着改了一项.
    cloud.changedElsewhere(snapshotOf({ quantity: 8800, coal_prefs: { 别人的煤: { enabled: true } } }));
    setCoalPref("甲", { enabled: true });

    await vi.waitFor(() => expect(getSyncState().status).toBe("conflict"));
    expect(cloud.puts).toHaveLength(0);
    expect(cloud.content.quantity).toBe(8800);
    expect(localStorage.getItem(KEY_SYNC_PENDING)).toBe("conflict");
    cleanup();
  });

  it("写回失败后别人改了云端, 点重试也不推, 进入冲突", async () => {
    const cloud = fakeCloud(snapshotOf({}));
    const cleanup = await initializeCloudStorage();
    cloud.failNext("PUT", "数据库忙");
    setCoalPref("甲", { enabled: true });
    await vi.waitFor(() => expect(getSyncState().status).toBe("error"));

    cloud.changedElsewhere(snapshotOf({ quantity: 8800 }));
    retrySync();

    await vi.waitFor(() => expect(getSyncState().status).toBe("conflict"));
    expect(cloud.puts).toHaveLength(0);
    expect(cloud.content.quantity).toBe(8800);
    cleanup();
  });

  it("连改两项, 前一笔成功后一笔失败: 基线跟到前一笔, 重新初始化不误报冲突", async () => {
    const cloud = fakeCloud(snapshotOf({}));
    let cleanup = await initializeCloudStorage();
    setCoalPref("甲", { enabled: true });
    await vi.waitFor(() => expect(cloud.puts).toHaveLength(1));
    cloud.failNext("PUT", "数据库忙");
    setCoalPref("乙", { enabled: true });
    await vi.waitFor(() => expect(getSyncState().status).toBe("error"));
    cleanup();

    // 刷新页面: 云端是第一笔的内容, 没有别人动过 → 应自动把本机 (含第二笔) 推上去.
    cleanup = await initializeCloudStorage();

    expect(getSyncState().status).toBe("idle");
    expect(hasUnsyncedChanges()).toBe(false);
    expect(Object.keys(cloud.content.coal_prefs).sort()).toEqual(["乙", "甲"]);
    cleanup();
  });

  it("写回在途时把值改回原值: 在途那笔成功后再补写一次, 本机与云端不分叉", async () => {
    const cloud = fakeCloud(snapshotOf({}));
    const cleanup = await initializeCloudStorage();
    const release = cloud.holdWrites();

    setShovelTons(5);
    await vi.waitFor(() => expect(cloud.puts).toHaveLength(1));
    setShovelTons(3);
    release();

    await vi.waitFor(() => expect(getSyncState().status).toBe("idle"));
    expect(cloud.content.shovel_tons).toBe(3);
    expect(hasUnsyncedChanges()).toBe(false);
    cleanup();
  });

  it("标记残留 (本机与云端其实一致): 只清标记, 不再整份推上去", async () => {
    // 上次 PUT 发出后页面被杀, 服务端已写成功, 成功回调没跑 —— 标记是白留下的.
    localStorage.setItem("doudou_blend.coal_prefs.v1", JSON.stringify({ 本机煤: { enabled: true } }));
    localStorage.setItem(KEY_SYNC_PENDING, "1");
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce(
      Response.json({
        initialized: true,
        coal_prefs: { 本机煤: { enabled: true } },
        contract: null,
        quantity: 3700,
        user_coals: [],
      }),
    );

    const cleanup = await initializeCloudStorage();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(hasUnsyncedChanges()).toBe(false);
    expect(getSyncState().status).toBe("idle");
    cleanup();
  });

  it("有标记且云端仍等于基线 (期间没人改过): 自动把本机推上去", async () => {
    const remote = snapshotOf({ coal_prefs: { 云端旧煤: { enabled: true } } });
    seedBaseline(remote);
    localStorage.setItem("doudou_blend.coal_prefs.v1", JSON.stringify({ 本机煤: { enabled: true } }));
    localStorage.setItem(KEY_SYNC_PENDING, "1");
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(Response.json({ initialized: true, ...remote }))
      .mockResolvedValueOnce(Response.json({ initialized: true }));

    const cleanup = await initializeCloudStorage();

    expect(getCoalPrefs()).toEqual({ 本机煤: { enabled: true } });
    expect(fetchMock.mock.calls[1]?.[1]?.method).toBe("PUT");
    expect(JSON.parse(fetchMock.mock.calls[1]?.[1]?.body as string).coal_prefs).toEqual({
      本机煤: { enabled: true },
    });
    expect(hasUnsyncedChanges()).toBe(false);
    // 基线跟着更新成本机这份: 下次再看, 这就是"上次与云端一致"的内容.
    expect(localStorage.getItem(KEY_SYNC_BASELINE)).toBe(
      canonicalUserSnapshot(getUserStorageSnapshot()),
    );
    cleanup();
  });

  it("有标记且云端仍等于基线, 推本机失败也不覆盖本机, 进入应用并提示未同步", async () => {
    const remote = snapshotOf({ coal_prefs: { 云端旧煤: { enabled: true } } });
    seedBaseline(remote);
    localStorage.setItem("doudou_blend.coal_prefs.v1", JSON.stringify({ 本机煤: { enabled: true } }));
    localStorage.setItem(KEY_SYNC_PENDING, "1");
    vi.mocked(fetch)
      .mockResolvedValueOnce(Response.json({ initialized: true, ...remote }))
      .mockResolvedValueOnce(new Response("", { status: 502 }));

    const cleanup = await initializeCloudStorage();

    expect(getCoalPrefs()).toEqual({ 本机煤: { enabled: true } });
    expect(getSyncState().status).toBe("error");
    expect(hasUnsyncedChanges()).toBe(true);
    cleanup();
  });

  it("历史导入失败不阻断登录, 不用 keepalive, 遗留历史留在本机", async () => {
    localStorage.setItem(
      "doudou_blend.history.v1",
      JSON.stringify([{ id: "legacy-1", occurred_at: "2026-07-29T00:00:00.000Z", cost_cif: 1, recipe: {}, contract_name: "" }]),
    );
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(
        Response.json({ initialized: true, coal_prefs: {}, contract: null, quantity: 3700, user_coals: [] }),
      )
      .mockRejectedValueOnce(new TypeError("Failed to fetch"));
    vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const cleanup = await initializeCloudStorage();

    expect(fetchMock.mock.calls[1]?.[0]).toBe("/api/history/import");
    expect(fetchMock.mock.calls[1]?.[1]?.keepalive).toBeUndefined();
    expect(localStorage.getItem("doudou_blend.history.v1")).not.toBeNull();
    cleanup();
  });

  /** 冲突现场: 有标记, 云端在标记期间从基线变成了别人改过的内容. */
  async function setupConflict() {
    seedBaseline(snapshotOf({ coal_prefs: { 我上次见的: { enabled: true } } }));
    localStorage.setItem("doudou_blend.coal_prefs.v1", JSON.stringify({ 本机煤: { enabled: true } }));
    localStorage.setItem(KEY_SYNC_PENDING, "1");
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce(
      Response.json({
        initialized: true,
        coal_prefs: { 别人改的: { enabled: true } },
        contract: null,
        quantity: 4200,
        user_coals: [],
      }),
    );
    const cleanup = await initializeCloudStorage();
    return { fetchMock, cleanup };
  }

  it("有标记期间云端被别人改过: 不自动推、不覆盖本机, 进入冲突等用户选", async () => {
    const { fetchMock, cleanup } = await setupConflict();

    expect(getSyncState().status).toBe("conflict");
    // 一个 PUT 都不许发
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(getCoalPrefs()).toEqual({ 本机煤: { enabled: true } });
    expect(hasUnsyncedChanges()).toBe(true);
    cleanup();
  });

  it("冲突没解决前, 用户继续做的改动也不写云端", async () => {
    const { fetchMock, cleanup } = await setupConflict();

    setCoalPref("后加的煤", { enabled: true });
    await new Promise((done) => setTimeout(done, 20));

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(getCoalPrefs()).toMatchObject({ 本机煤: { enabled: true }, 后加的煤: { enabled: true } });
    cleanup();
  });

  it("冲突选「用云端」: 重新拉一次云端再覆盖本机, 不写回, 标记清掉", async () => {
    const { fetchMock, cleanup } = await setupConflict();
    // 横幅挂着的这段时间, 云端又被改了一次: 要套用的是现在的云端, 不是初始化时拉到的那份.
    const latest = snapshotOf({ quantity: 5000, coal_prefs: { 又改了一次: { enabled: true } } });
    fetchMock.mockResolvedValueOnce(Response.json({ initialized: true, ...latest }));

    await resolveSyncConflict("cloud");

    expect(getCoalPrefs()).toEqual({ 又改了一次: { enabled: true } });
    expect(localStorage.getItem("doudou_blend.quantity.v1")).toBe("5000");
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[1]?.[1]?.method).toBeUndefined();
    expect(localStorage.getItem(KEY_SYNC_BASELINE)).toBe(canonicalUserSnapshot(latest));
    expect(hasUnsyncedChanges()).toBe(false);
    expect(getSyncState().status).toBe("idle");
    cleanup();
  });

  it("冲突选「用云端」拉不到云端: 留在冲突状态, 本机不动", async () => {
    const { fetchMock, cleanup } = await setupConflict();
    fetchMock.mockResolvedValueOnce(
      new Response('{"ok":false,"reason":"数据库忙"}', { status: 503 }),
    );

    await resolveSyncConflict("cloud");

    expect(getSyncState()).toEqual({ status: "conflict", error: "数据库忙" });
    expect(getCoalPrefs()).toEqual({ 本机煤: { enabled: true } });
    cleanup();
  });

  it("冲突选「用本机」的写回在途时又做的改动: 不算已同步, 写完后补写", async () => {
    const { fetchMock, cleanup } = await setupConflict();
    let release!: () => void;
    const held = new Promise<void>((done) => {
      release = done;
    });
    const bodies: UserStorageSnapshot[] = [];
    let cloud: UserStorageSnapshot | null = null;
    fetchMock.mockImplementation(async (_input, init) => {
      if (init?.method !== "PUT") return Response.json({ initialized: true, ...(cloud ?? snapshotOf({})) });
      const body = JSON.parse(init.body as string) as UserStorageSnapshot;
      bodies.push(body);
      if (bodies.length === 1) await held;
      cloud = body;
      return Response.json({ initialized: true });
    });

    const resolved = resolveSyncConflict("local");
    await vi.waitFor(() => expect(bodies).toHaveLength(1));
    setCoalPref("中途加的煤", { enabled: true });
    release();
    await resolved;

    await vi.waitFor(() => expect(bodies).toHaveLength(2));
    expect(bodies[0]?.coal_prefs).toEqual({ 本机煤: { enabled: true } });
    expect(bodies[1]?.coal_prefs).toMatchObject({ 中途加的煤: { enabled: true } });
    await vi.waitFor(() => expect(getSyncState().status).toBe("idle"));
    expect(hasUnsyncedChanges()).toBe(false);
    cleanup();
  });

  it("冲突选「用本机」: 覆盖云端, 基线更新", async () => {
    const { fetchMock, cleanup } = await setupConflict();
    fetchMock.mockResolvedValueOnce(Response.json({ initialized: true }));

    await resolveSyncConflict("local");

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[1]?.[1]?.method).toBe("PUT");
    expect(JSON.parse(fetchMock.mock.calls[1]?.[1]?.body as string).coal_prefs).toEqual({
      本机煤: { enabled: true },
    });
    expect(getCoalPrefs()).toEqual({ 本机煤: { enabled: true } });
    expect(hasUnsyncedChanges()).toBe(false);
    expect(getSyncState().status).toBe("idle");
    expect(localStorage.getItem(KEY_SYNC_BASELINE)).toBe(
      canonicalUserSnapshot(getUserStorageSnapshot()),
    );
    cleanup();
  });

  it("冲突选「用本机」写不出去: 留在冲突状态并说明原因", async () => {
    const { fetchMock, cleanup } = await setupConflict();
    fetchMock.mockResolvedValueOnce(
      new Response('{"ok":false,"reason":"数据库忙"}', { status: 503 }),
    );

    await resolveSyncConflict("local");

    expect(getSyncState()).toEqual({ status: "conflict", error: "数据库忙" });
    expect(getCoalPrefs()).toEqual({ 本机煤: { enabled: true } });
    expect(hasUnsyncedChanges()).toBe(true);
    cleanup();
  });

  it("升级前留下的标记 (没有基线) 且本机与云端不同: 按云端被改过处理", async () => {
    localStorage.setItem("doudou_blend.coal_prefs.v1", JSON.stringify({ 本机煤: { enabled: true } }));
    localStorage.setItem(KEY_SYNC_PENDING, "1");
    const fetchMock = vi.mocked(fetch);
    fetchMock.mockResolvedValueOnce(
      Response.json({
        initialized: true,
        coal_prefs: { 云端煤: { enabled: true } },
        contract: null,
        quantity: 3700,
        user_coals: [],
      }),
    );

    const cleanup = await initializeCloudStorage();

    expect(getSyncState().status).toBe("conflict");
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(getCoalPrefs()).toEqual({ 本机煤: { enabled: true } });
    cleanup();
  });

  it("写回失败后重新初始化 (退出再登录), 同步状态复位, 不会再推旧快照", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(
        Response.json({ initialized: true, coal_prefs: {}, contract: null, quantity: 3700, user_coals: [] }),
      )
      .mockResolvedValueOnce(new Response('{"ok":false,"reason":"数据库忙"}', { status: 503 }))
      .mockResolvedValueOnce(
        Response.json({ initialized: true, coal_prefs: {}, contract: null, quantity: 3700, user_coals: [] }),
      );

    const first = await initializeCloudStorage();
    setCoalPref("甲", { enabled: true });
    await vi.waitFor(() => expect(getSyncState().status).toBe("error"));
    first();

    // 模拟退出再登录: 本机数据已被清, 重新初始化后状态必须回到与实际一致.
    localStorage.clear();
    const second = await initializeCloudStorage();

    expect(getSyncState()).toEqual({ status: "idle", error: null });
    expect(hasUnsyncedChanges()).toBe(false);
    retrySync();
    await new Promise((done) => setTimeout(done, 20));
    expect(fetchMock).toHaveBeenCalledTimes(3);
    second();
  });

  it("resetSyncState 把出错状态拉回 idle, 重试也不再发请求", async () => {
    const fetchMock = vi.mocked(fetch);
    fetchMock
      .mockResolvedValueOnce(
        Response.json({ initialized: true, coal_prefs: {}, contract: null, quantity: 3700, user_coals: [] }),
      )
      .mockResolvedValueOnce(new Response('{"ok":false,"reason":"数据库忙"}', { status: 503 }));
    const cleanup = await initializeCloudStorage();
    setCoalPref("甲", { enabled: true });
    await vi.waitFor(() => expect(getSyncState().status).toBe("error"));

    resetSyncState();

    expect(getSyncState()).toEqual({ status: "idle", error: null });
    retrySync();
    await new Promise((done) => setTimeout(done, 20));
    expect(fetchMock).toHaveBeenCalledTimes(2);
    cleanup();
  });

  it("写回请求体小时用 keepalive, 超过 64KiB 限制时不用", async () => {
    const cloud = fakeCloud(snapshotOf({}));
    const cleanup = await initializeCloudStorage();

    setCoalPref("甲", { enabled: true });
    await vi.waitFor(() => expect(cloud.puts).toHaveLength(1));
    expect(cloud.puts[0]?.keepalive).toBe(true);

    setCoalPref("乙", { props_override: Object.fromEntries(Array.from({ length: 8000 }, (_, i) => [`k${i}`, i])) });
    await vi.waitFor(() => expect(cloud.puts).toHaveLength(2));
    expect(cloud.puts[1]?.keepalive).toBe(false);
    cleanup();
  });
});
