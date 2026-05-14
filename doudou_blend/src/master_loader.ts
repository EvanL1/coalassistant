/** 数据加载层: 从后端 (WASM 或 Tauri) 取 master 数据并 cache. */

import { getBackend } from "./backend";
import type { CoalMaster } from "./types";

let masterCache: CoalMaster | null = null;

export async function loadMaster(): Promise<CoalMaster> {
  if (masterCache) return masterCache;
  const backend = await getBackend();
  const json = await backend.getMasterJson();
  masterCache = JSON.parse(json) as CoalMaster;
  return masterCache;
}

/** 强制刷新 master cache (用户改了 master 后调用). */
export function invalidateMaster() {
  masterCache = null;
}
