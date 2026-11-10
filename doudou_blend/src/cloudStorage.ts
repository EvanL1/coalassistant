import {
  canonicalUserSnapshot,
  clearHistory,
  DEFAULT_SHOVEL_TONS,
  getHistory,
  getUserStorageSnapshot,
  replaceUserStorageSnapshot,
  USER_STORAGE_EVENTS,
  type UserStorageSnapshot,
} from "./storage";
import { apiFetch, errorMessage, readOk } from "./http";

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
export const SYNC_STATUS_EVENT = "doudou:sync_status_changed";

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
  window.dispatchEvent(new CustomEvent(SYNC_STATUS_EVENT));
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

async function request(path: string, init?: RequestInit): Promise<string> {
  return readOk(
    await apiFetch(`/api/${path}`, {
      credentials: "same-origin",
      ...init,
    }),
  );
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
  await request("storage", {
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
  await request("history/import", {
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
  const remote = parseRemoteStorage(JSON.parse(await request("storage", { cache: "no-store" })));
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
  if (!remote.initialized) {
    const local = getUserStorageSnapshot();
    await saveRemoteStorage(local);
    setBaseline(canonicalUserSnapshot(local));
    setPendingFlag(null);
  } else if (hasUnsyncedChanges()) {
    const local = getUserStorageSnapshot();
    const localCanon = canonicalUserSnapshot(local);
    if (localCanon === remote.canon) {
      // 上次 PUT 其实写成功了, 只是成功回调没跑 (页面被系统杀掉): 标记白留了, 清掉即可.
      setPendingFlag(null);
      setBaseline(remote.canon);
    } else if (getBaseline() === remote.canon) {
      // 标记期间云端没人动过: 本机的改动比云端新, 自动推上去; 推不上去也不拿云端旧值覆盖本机.
      try {
        await saveRemoteStorage(local);
        setPendingFlag(null);
        setBaseline(localCanon);
      } catch (error) {
        setSyncState("error", errorMessage(error));
      }
    } else {
      // 云端在标记期间被别人改过 (或升级前留下的标记, 没有基线可比对, 按被别人改过处理):
      // 不自动推也不覆盖本机, 等用户在横幅里二选一; 选之前一个字节都不往云端写.
      enterConflict();
    }
  } else {
    replaceUserStorageSnapshot(remote.snapshot);
    setBaseline(remote.canon);
    setPendingFlag(null);
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
    // 冲突没选完之前 (本标签页或同一浏览器的别的标签页), 改动只留在本机.
    if (conflict || pendingFlag() === CONFLICT) {
      if (!conflict) enterConflict();
      return;
    }
    const snapshot = getUserStorageSnapshot();
    const canon = canonicalUserSnapshot(snapshot);
    // 内容与云端一致、也没有写回在途 (例如冲突选「用云端」后的事件回波): 没有要写的.
    // 有写回在途时不能走这条: 在途那笔成功后云端是它的内容, 本机得再写一次才一致.
    if (inFlight === 0 && canon === getBaseline()) {
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
        // 写之前先看云端还停不停在基线上: 页面可能开了很久, 别人已经改过云端,
        // 这时整份推上去会抹掉对方的改动. 没有服务端版本号, 拉与写之间仍有一次往返的窗口.
        const remote = await fetchRemote();
        if (remote.canon !== canon) {
          if (remote.canon !== getBaseline()) {
            enterConflict();
            return;
          }
          await saveRemoteStorage(snapshot);
        }
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
  for (const eventName of USER_STORAGE_EVENTS) {
    window.addEventListener(eventName, persist);
  }

  return () => {
    if (retryWrite === persist) retryWrite = null;
    for (const eventName of USER_STORAGE_EVENTS) {
      window.removeEventListener(eventName, persist);
    }
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
