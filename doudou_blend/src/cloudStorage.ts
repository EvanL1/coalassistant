import {
  clearHistory,
  getHistory,
  getUserStorageSnapshot,
  replaceUserStorageSnapshot,
  USER_STORAGE_EVENTS,
  type UserStorageSnapshot,
} from "./storage";

interface RemoteUserStorage extends UserStorageSnapshot {
  initialized: boolean;
}

async function request(path: string, init?: RequestInit): Promise<Response> {
  const response = await fetch(`/api/${path}`, {
    credentials: "same-origin",
    ...init,
  });
  if (!response.ok) {
    const body = await response.text();
    throw new Error(body || `HTTP ${response.status}`);
  }
  return response;
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
    !Array.isArray(storage.user_coals)
  ) {
    throw new Error("服务端用户状态字段无效");
  }
  return storage as RemoteUserStorage;
}

async function saveRemoteStorage(snapshot: UserStorageSnapshot): Promise<void> {
  await request("storage", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(snapshot),
    keepalive: true,
  });
}

async function importLocalHistory(): Promise<void> {
  const entries = getHistory();
  if (entries.length === 0) return;
  await request("history/import", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ entries }),
    keepalive: true,
  });
  clearHistory();
}

/**
 * 登录后加载 PostgreSQL 状态并启动本地缓存写回。
 * 返回清理函数。
 */
export async function initializeCloudStorage(): Promise<() => void> {
  const response = await request("storage", { cache: "no-store" });
  const remote = parseRemoteStorage(await response.json());
  if (remote.initialized) {
    replaceUserStorageSnapshot(remote);
  } else {
    await saveRemoteStorage(getUserStorageSnapshot());
  }
  await importLocalHistory();

  let writeQueue: Promise<void> = Promise.resolve();
  const persist = () => {
    const snapshot = getUserStorageSnapshot();
    writeQueue = writeQueue
      .catch(() => undefined)
      .then(() => saveRemoteStorage(snapshot))
      .catch((error: unknown) => {
        window.dispatchEvent(
          new CustomEvent("doudou:storage_sync_error", { detail: error }),
        );
      });
  };
  for (const eventName of USER_STORAGE_EVENTS) {
    window.addEventListener(eventName, persist);
  }

  return () => {
    for (const eventName of USER_STORAGE_EVENTS) {
      window.removeEventListener(eventName, persist);
    }
  };
}
