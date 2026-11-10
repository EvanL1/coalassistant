/** 数据加载层: 从后端 HTTP API 取 master 数据并 cache. */

import { getBackend } from "./backend";
import type { CoalMaster } from "./types";

let masterCache: Promise<CoalMaster> | null = null;

/** 并发调用共用同一个请求; 失败时清掉缓存, 下次调用重新请求. */
export function loadMaster(): Promise<CoalMaster> {
  if (masterCache) return masterCache;
  const pending = (async () => {
    const backend = await getBackend();
    const json = await backend.getMasterJson();
    return JSON.parse(json) as CoalMaster;
  })();
  masterCache = pending;
  pending.catch(() => {
    if (masterCache === pending) masterCache = null;
  });
  return pending;
}

/** 强制刷新 master cache (用户改了 master 后调用). */
export function invalidateMaster() {
  masterCache = null;
}
