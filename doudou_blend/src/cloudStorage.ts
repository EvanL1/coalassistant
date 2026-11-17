import {
  canonicalUserSnapshot,
  clearHistory,
  DEFAULT_SHOVEL_TONS,
  getHistory,
  getUserStorageSnapshot,
  replaceUserStorageSnapshot,
  type UserStorageSnapshot,
} from "./storage";
import { emit, listen, USER_STORAGE_EVENTS } from "./events";
import { apiJson, errorMessage } from "./http";

/*
 * 云端同步: 本机 localStorage 是同步缓存, 这里把它镜像到 PostgreSQL (GET/PUT /api/storage).
 *
 * "该推、该拉、还是算冲突" 的判断是下面三个纯函数 (decideInit / decidePersist / decideWrite),
 * 只看传进来的值, 不读写任何东西; 读写 localStorage、发请求、派事件都在后面的外壳里.
 *
 * 已知没覆盖的情况 (现状, 本文件没有处理; 要彻底堵住得给服务端加版本号):
 *   1. 写成功但回包丢了: 基线没推进. 之后本机再改一笔, 写前拉到的云端是上一笔自己写的内容,
 *      既不等于这一笔也不等于基线 —— 会误报冲突 (见 decideWrite).
 *   2. 两台设备同时首次登录: 两边拉到的都是"云端没初始化", 各自把本机整份推上去,
 *      后到的覆盖先到的, 没有任何提示 (见 decideInit 的 seed_cloud).
 *   3. 多标签页: 事件只在本窗口派发, 别的标签页改了数据这边的界面不会刷新;
 *      在途写回的计数 (inFlight) 也是各标签页各记各的.
 *   4. 重新初始化 (再次登录) 不取消上一轮已经排进队列的写回: 它们照样跑完,
 *      之后还会改基线、未同步标记和同步状态.
 */
interface RemoteUserStorage extends UserStorageSnapshot {
  initialized: boolean;
}

/**
 * 本机有改动还没写进云端; 持久化, 下次登录据此判断本机比云端新.
 * 值 "1" = 有待写的改动; "conflict" = 云端被别人改过、等用户二选一 (同一浏览器的别的
 * 标签页也看得到, 选之前谁都不往云端写).
 */
const KEY_SYNC_PENDING = "doudou_blend.sync_pending.v1";
const PENDING = "1";
const CONFLICT = "conflict";
/**
 * 上次确认与云端一致时的快照 (稳定序列化, 见 storage.canonicalUserSnapshot);
 * 每次从云端拉取成功、每次写回成功后更新. 靠它分辨"云端期间有没有被别人改过" ——
 * 线上多人多设备共用一行数据, 不能看见本机有改动就整份推.
 */
const KEY_SYNC_BASELINE = "doudou_blend.sync_baseline.v1";
/** 浏览器对 keepalive 请求体限 64KiB, 超了直接报错; 留点余量. */
const KEEPALIVE_MAX_BYTES = 60 * 1024;

export type SyncStatus = "idle" | "syncing" | "error" | "conflict";

// ============================================================
// 决策 (纯函数)
// ============================================================

/** 稳定序列化后的快照 (storage.canonicalUserSnapshot), 只用来比"是不是同一份". */
type Canon = string;

export type InitAction =
  /** 云端还没初始化: 把本机整份推上去当第一份. */
  | "seed_cloud"
  /** 本机没有未同步的改动: 用云端覆盖本机. */
  | "adopt_cloud"
  /** 有未同步标记但本机与云端相同 (上次其实写成功了): 只清标记、记基线. */
  | "already_in_sync"
  /** 标记期间云端没人动过: 本机比云端新, 推上去. */
  | "push_local"
  /** 云端在标记期间被别人改过, 或没有基线可比: 不推也不覆盖, 等用户二选一. */
  | "conflict";

/** 登录后第一次对账: 本机与云端谁覆盖谁. */
export function decideInit(input: {
  /** 云端这一行写过没有. */
  initialized: boolean;
  /** 未同步标记: null = 没有; 其余 (有待写的改动 / 冲突没选完) 都算有. */
  pending: string | null;
  /** 上次确认与云端一致时的快照; null = 没记过 (升级前的客户端, 或存不进去). */
  baseline: Canon | null;
  local: Canon;
  remote: Canon;
}): InitAction {
  if (!input.initialized) return "seed_cloud";
  if (input.pending == null) return "adopt_cloud";
  if (input.local === input.remote) return "already_in_sync";
  if (input.baseline === input.remote) return "push_local";
  return "conflict";
}

export type PersistAction =
  /** 本标签页的冲突还没选完: 改动只留在本机. */
  | "hold"
  /** 同一浏览器的别的标签页标了冲突: 这边也进入冲突状态, 改动只留在本机. */
  | "join_conflict"
  /** 内容与基线一致、也没有写回在途: 没有要写的, 清标记. */
  | "in_sync"
  /** 排一笔写回. */
  | "write";

/** 本机数据变了 (或用户点重试): 要不要往云端写. */
export function decidePersist(input: {
  /** 本标签页是否处于冲突状态. */
  conflict: boolean;
  pending: string | null;
  /** 还没结束的写回笔数. */
  inFlight: number;
  next: Canon;
  baseline: Canon | null;
}): PersistAction {
  if (input.conflict) return "hold";
  if (input.pending === CONFLICT) return "join_conflict";
  // 有写回在途时不能算一致: 在途那笔成功后云端是它的内容, 本机得再写一次才一致.
  if (input.inFlight === 0 && input.next === input.baseline) return "in_sync";
  return "write";
}

export type WriteAction =
  /** 云端已经是要写的内容: 不用发请求, 只推进基线. */
  | "already_written"
  /** 云端还停在基线上: 推上去. */
  | "push"
  /** 云端既不是要写的内容也不在基线上: 被别人改过, 不推. */
  | "conflict";

/**
 * 轮到一笔写回时, 先拉一次云端再决定: 页面可能开了很久, 别人已经改过云端,
 * 这时整份推上去会抹掉对方的改动. 没有服务端版本号, 拉与写之间仍有一次往返的窗口.
 */
export function decideWrite(input: {
  remote: Canon;
  next: Canon;
  baseline: Canon | null;
}): WriteAction {
  if (input.remote === input.next) return "already_written";
  if (input.remote === input.baseline) return "push";
  return "conflict";
}

// ============================================================
// 外壳: 读写 localStorage、发请求、派事件
// ============================================================
let syncStatus: SyncStatus = "idle";
let syncError: string | null = null;
let retryWrite: (() => void) | null = null;
/** 云端被别人改过: 用户二选一之前挂起所有写回. */
let conflict = false;
/** 二选一的请求在途: 期间不接第二次选择. */
let resolving = false;

export function getSyncState(): { status: SyncStatus; error: string | null } {
  return { status: syncStatus, error: syncError };
}

function setSyncState(status: SyncStatus, error: string | null = null): void {
  syncStatus = status;
  syncError = error;
  emit("sync_status_changed");
}

/** 登出、会话失效、重新初始化时调用: 同步状态回到与实际一致, 重试不再推旧快照. */
export function resetSyncState(): void {
  retryWrite = null;
  conflict = false;
  resolving = false;
  setSyncState("idle");
}

function pendingFlag(): string | null {
  try {
    return localStorage.getItem(KEY_SYNC_PENDING);
  } catch {
    return null;
  }
}

export function hasUnsyncedChanges(): boolean {
  return pendingFlag() != null;
}

function setPendingFlag(value: typeof PENDING | typeof CONFLICT | null): void {
  try {
    if (value == null) localStorage.removeItem(KEY_SYNC_PENDING);
    else localStorage.setItem(KEY_SYNC_PENDING, value);
  } catch {
    // 存不进标记时退回旧行为: 下次登录以云端为准 (只会丢本机自己的改动, 不会盖别人的)
  }
}

function getBaseline(): string | null {
  try {
    return localStorage.getItem(KEY_SYNC_BASELINE);
  } catch {
    return null;
  }
}

function setBaseline(canon: string): void {
  try {
    localStorage.setItem(KEY_SYNC_BASELINE, canon);
  } catch {
    // 存不进新基线时把旧的也删掉: 没有基线就一律按"云端被改过"处理, 不会误推.
    try {
      localStorage.removeItem(KEY_SYNC_BASELINE);
    } catch {
      // 存储整个不可用, 没有可做的
    }
  }
}

function enterConflict(error: string | null = null): void {
  conflict = true;
  setPendingFlag(CONFLICT);
  setSyncState("conflict", error);
}

/** 把本机当前数据重新写一次云端; 同步没启动时什么也不做. */
export function retrySync(): void {
  retryWrite?.();
}

function parseRemoteStorage(value: unknown): RemoteUserStorage {
  if (value == null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("服务端用户状态格式无效");
  }
  const storage = value as Partial<RemoteUserStorage>;
  if (
    typeof storage.initialized !== "boolean" ||
    storage.coal_prefs == null ||
    typeof storage.coal_prefs !== "object" ||
    Array.isArray(storage.coal_prefs) ||
    (storage.contract != null && !Array.isArray(storage.contract)) ||
    typeof storage.quantity !== "number" ||
    !Number.isFinite(storage.quantity) ||
    storage.quantity <= 0 ||
    !Array.isArray(storage.user_coals) ||
    (storage.shovel_tons != null &&
      (typeof storage.shovel_tons !== "number" ||
        !Number.isFinite(storage.shovel_tons) ||
        storage.shovel_tons <= 0))
  ) {
    throw new Error("服务端用户状态字段无效");
  }
  // 一铲吨数上线前部署的服务端不回这个字段.
  return {
    ...storage,
    shovel_tons: storage.shovel_tons ?? DEFAULT_SHOVEL_TONS,
  } as RemoteUserStorage;
}

function byteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

async function saveRemoteStorage(snapshot: UserStorageSnapshot): Promise<void> {
  const body = JSON.stringify(snapshot);
  await apiJson("/api/storage", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body,
    // keepalive 让关页面前的最后一次写回也能发出去, 但请求体超限时浏览器直接拒发.
    keepalive: byteLength(body) <= KEEPALIVE_MAX_BYTES,
  });
}

async function importLocalHistory(): Promise<void> {
  const entries = getHistory();
  if (entries.length === 0) return;
  await apiJson("/api/history/import", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ entries }),
  });
  clearHistory();
}

/** 拉一次云端, 连同它的稳定序列化 (与基线、本机比对用). */
async function fetchRemote(): Promise<{
  initialized: boolean;
  snapshot: UserStorageSnapshot;
  canon: string;
}> {
  const remote = parseRemoteStorage(await apiJson<unknown>("/api/storage", { cache: "no-store" }));
  const snapshot: UserStorageSnapshot = {
    coal_prefs: remote.coal_prefs,
    contract: remote.contract ?? null,
    quantity: remote.quantity,
    user_coals: remote.user_coals,
    shovel_tons: remote.shovel_tons,
  };
  return { initialized: remote.initialized, snapshot, canon: canonicalUserSnapshot(snapshot) };
}

/**
 * 登录后加载 PostgreSQL 状态并启动本地缓存写回。
 * 返回清理函数。
 */
export async function initializeCloudStorage(): Promise<() => void> {
  resetSyncState();
  const remote = await fetchRemote();
  const local = getUserStorageSnapshot();
  const localCanon = canonicalUserSnapshot(local);
  switch (
    decideInit({
      initialized: remote.initialized,
      pending: pendingFlag(),
      baseline: getBaseline(),
      local: localCanon,
      remote: remote.canon,
    })
  ) {
    case "seed_cloud":
      await saveRemoteStorage(local);
      setBaseline(localCanon);
      setPendingFlag(null);
      break;
    case "already_in_sync":
      // 上次 PUT 其实写成功了, 只是成功回调没跑 (页面被系统杀掉): 标记白留了, 清掉即可.
      setPendingFlag(null);
      setBaseline(remote.canon);
      break;
    case "push_local":
      // 推不上去也不拿云端旧值覆盖本机.
      try {
        await saveRemoteStorage(local);
        setPendingFlag(null);
        setBaseline(localCanon);
      } catch (error) {
        setSyncState("error", errorMessage(error));
      }
      break;
    case "conflict":
      // 选之前一个字节都不往云端写.
      enterConflict();
      break;
    case "adopt_cloud":
      replaceUserStorageSnapshot(remote.snapshot);
      setBaseline(remote.canon);
      setPendingFlag(null);
      break;
  }
  try {
    await importLocalHistory();
  } catch (error) {
    // 遗留历史留在本机, 下次登录再导; 不能因此卡住登录.
    console.warn("本机遗留历史导入失败", error);
  }

  let writeQueue: Promise<void> = Promise.resolve();
  let latestWrite = 0;
  let inFlight = 0;
  const persist = () => {
    const snapshot = getUserStorageSnapshot();
    const canon = canonicalUserSnapshot(snapshot);
    const action = decidePersist({
      conflict,
      pending: pendingFlag(),
      inFlight,
      next: canon,
      baseline: getBaseline(),
    });
    if (action === "hold") return;
    if (action === "join_conflict") {
      enterConflict();
      return;
    }
    if (action === "in_sync") {
      // 例如冲突选「用云端」后的事件回波.
      setPendingFlag(null);
      setSyncState("idle");
      return;
    }
    const write = ++latestWrite;
    setPendingFlag(PENDING);
    setSyncState("syncing");
    inFlight += 1;
    writeQueue = writeQueue
      .catch(() => undefined)
      .then(async () => {
        if (conflict) return;
        const remote = await fetchRemote();
        const step = decideWrite({ remote: remote.canon, next: canon, baseline: getBaseline() });
        if (step === "conflict") {
          enterConflict();
          return;
        }
        if (step === "push") await saveRemoteStorage(snapshot);
        // 每笔写成功都推进基线 (队列是串行的): 不然后一笔失败时基线落后于云端,
        // 下次初始化会把自己写的内容当成"别人改过".
        setBaseline(canon);
        if (write !== latestWrite) return;
        setPendingFlag(null);
        setSyncState("idle");
      })
      .catch((error: unknown) => {
        if (write === latestWrite && !conflict) setSyncState("error", errorMessage(error));
      })
      .finally(() => {
        inFlight -= 1;
      });
  };
  retryWrite = persist;
  const stopListening = listen(USER_STORAGE_EVENTS, persist);

  return () => {
    if (retryWrite === persist) retryWrite = null;
    stopListening();
  };
}

/**
 * 冲突二选一: 「用云端」放弃本机没同步的改动 (含冲突期间接着做的); 「用本机」覆盖云端
 * (另一台设备的改动会丢). 选择在横幅里做出, 见 SyncBanner.
 */
export async function resolveSyncConflict(choice: "local" | "cloud"): Promise<void> {
  if (!conflict || resolving) return;
  resolving = true;
  try {
    if (choice === "cloud") {
      // 重新拉一次: 横幅挂着的这段时间云端可能又变了, 套用初始化时拉到的旧值
      // 会让下一笔改动把更新的云端盖掉.
      const remote = await fetchRemote();
      // conflict 先留着, replace 派发的事件才不会触发一次回写.
      replaceUserStorageSnapshot(remote.snapshot);
      setBaseline(remote.canon);
      conflict = false;
      setPendingFlag(null);
      setSyncState("idle");
      return;
    }
    const snapshot = getUserStorageSnapshot();
    const canon = canonicalUserSnapshot(snapshot);
    await saveRemoteStorage(snapshot);
    setBaseline(canon);
    conflict = false;
    setPendingFlag(null);
    setSyncState("idle");
    // 写回在途期间用户又做的改动没在这次请求里: 补写一次.
    if (canonicalUserSnapshot(getUserStorageSnapshot()) !== canon) retryWrite?.();
  } catch (error) {
    // 拉不到或写不出去就留在冲突状态, 说清原因, 用户可以再选.
    setSyncState("conflict", errorMessage(error));
  } finally {
    resolving = false;
  }
}
