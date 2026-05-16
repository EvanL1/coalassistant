/**
 * 用户数据存储层.
 *
 * Web 端: localStorage
 * Tauri 端: 后续接 SQLite (user_overrides / user_coal_prefs 表) - 当前先 fallback localStorage
 *
 * 设计:
 *   - master 是只读的(blend_kit_rs 嵌入 JSON)
 *   - 用户改的所有东西都存在这里
 *   - 重置 = 清空 localStorage 对应 key
 */

import type { Spec, MasterCoalEntry } from "./types";
import {
  apiAddCoal,
  apiDeleteCoal,
  apiListCoals,
  apiMigrateCoals,
  clearApiToken,
  getApiToken,
  setApiToken,
} from "./api";

const KEY_COAL_PREFS = "doudou_blend.coal_prefs.v1";
const KEY_CONTRACT = "doudou_blend.contract.v1";
const KEY_HISTORY = "doudou_blend.history.v1";
const KEY_AUTH = "doudou_blend.auth.v1";
const KEY_USER_COALS = "doudou_blend.user_coals.v1";
const KEY_USER_COALS_MIGRATED = "doudou_blend.user_coals_migrated.v1";

/** 单个煤的用户偏好: 启用 + 价格覆盖 + 化验值覆盖 */
export interface CoalPref {
  enabled: boolean;
  /** 用户改后的 FOB; null = 用 master 默认 */
  fob_override?: number | null;
  /** 用户改后的运费; null = 用 master 默认 */
  frt_override?: number | null;
  /** 用户改过的化验项; null = 用 master 默认 */
  props_override?: Partial<Record<string, number>>;
  /** 最近一次修改时间 (ISO) */
  updated_at?: string;
}

/** 全部煤的偏好 dict, key = coal name */
export type CoalPrefs = Record<string, CoalPref>;

/** 单条历史方案 (求解结果 + 时间戳) */
export interface HistoryEntry {
  id: string;          // crypto.randomUUID() 或时间戳
  occurred_at: string; // ISO
  cost_cif: number;
  recipe: Record<string, number>;
  contract_name: string;
  note?: string;
}

// ============================================================
// 煤偏好
// ============================================================

export function getCoalPrefs(): CoalPrefs {
  try {
    const raw = localStorage.getItem(KEY_COAL_PREFS);
    return raw ? (JSON.parse(raw) as CoalPrefs) : {};
  } catch {
    return {};
  }
}

export function setCoalPref(name: string, pref: Partial<CoalPref>): void {
  const all = getCoalPrefs();
  all[name] = {
    ...all[name],
    ...pref,
    updated_at: new Date().toISOString(),
  } as CoalPref;
  localStorage.setItem(KEY_COAL_PREFS, JSON.stringify(all));
  // 派发自定义事件让其他组件订阅
  window.dispatchEvent(new CustomEvent("doudou:prefs_changed"));
}

export function getCoalPref(name: string): CoalPref | null {
  return getCoalPrefs()[name] ?? null;
}

export function clearCoalPref(name: string): void {
  const all = getCoalPrefs();
  delete all[name];
  localStorage.setItem(KEY_COAL_PREFS, JSON.stringify(all));
  window.dispatchEvent(new CustomEvent("doudou:prefs_changed"));
}

export function clearAllCoalPrefs(): void {
  localStorage.removeItem(KEY_COAL_PREFS);
  window.dispatchEvent(new CustomEvent("doudou:prefs_changed"));
}

// ============================================================
// 合同
// ============================================================

export function getUserContract(): Spec[] | null {
  try {
    const raw = localStorage.getItem(KEY_CONTRACT);
    return raw ? (JSON.parse(raw) as Spec[]) : null;
  } catch {
    return null;
  }
}

export function setUserContract(specs: Spec[]): void {
  localStorage.setItem(KEY_CONTRACT, JSON.stringify(specs));
  window.dispatchEvent(new CustomEvent("doudou:contract_changed"));
}

export function clearUserContract(): void {
  localStorage.removeItem(KEY_CONTRACT);
  window.dispatchEvent(new CustomEvent("doudou:contract_changed"));
}

// ============================================================
// 历史方案
// ============================================================

export function getHistory(): HistoryEntry[] {
  try {
    const raw = localStorage.getItem(KEY_HISTORY);
    return raw ? (JSON.parse(raw) as HistoryEntry[]) : [];
  } catch {
    return [];
  }
}

export function appendHistory(entry: Omit<HistoryEntry, "id" | "occurred_at">): HistoryEntry {
  const full: HistoryEntry = {
    ...entry,
    id: crypto.randomUUID?.() ?? `${Date.now()}-${Math.random()}`,
    occurred_at: new Date().toISOString(),
  };
  const all = getHistory();
  all.unshift(full); // 最新在前
  // 保留最近 100 条避免无限增长
  if (all.length > 100) all.length = 100;
  localStorage.setItem(KEY_HISTORY, JSON.stringify(all));
  window.dispatchEvent(new CustomEvent("doudou:history_changed"));
  return full;
}

export function clearHistory(): void {
  localStorage.removeItem(KEY_HISTORY);
  window.dispatchEvent(new CustomEvent("doudou:history_changed"));
}

// ============================================================
// 用户新增的煤种 - 上 Cloudflare D1 后跨设备共享
// ============================================================
//
// 数据流: localStorage (cache) ─► UI 立即渲染
//                 │
//                 ▼ 启动时后台刷新
//        Cloudflare D1 (source of truth)
//
// - getUserCoals() 同步返回 cache, 用法跟以前不变 (UI 无感知)
// - addUserCoal / removeUserCoal 改为 async, 调用方需 await
// - 启动调一次 refreshUserCoals() 把服务器最新拉下来更新 cache
// - 跨设备: A 设备加煤 → D1 → B 设备打开应用时 refresh → 看到
// - 离线: 写操作失败 throw, 调用方提示用户; 读操作走 cache

function readCache(): MasterCoalEntry[] {
  try {
    const raw = localStorage.getItem(KEY_USER_COALS);
    return raw ? (JSON.parse(raw) as MasterCoalEntry[]) : [];
  } catch {
    return [];
  }
}

function writeCache(coals: MasterCoalEntry[]): void {
  localStorage.setItem(KEY_USER_COALS, JSON.stringify(coals));
}

/** 同步: 返回最后已知的 user_coals (localStorage cache). UI 用. */
export function getUserCoals(): MasterCoalEntry[] {
  return readCache();
}

/**
 * 异步: 从 D1 拉最新, 更新 cache 并派发事件.
 * 失败 (离线/未登录) 时静默返回 cache, 不打断 UI.
 */
export async function refreshUserCoals(): Promise<MasterCoalEntry[]> {
  if (!getApiToken()) return readCache();
  try {
    const remote = await apiListCoals();
    writeCache(remote);
    window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
    return remote;
  } catch (e) {
    console.warn("refreshUserCoals 失败, 使用 cache:", e);
    return readCache();
  }
}

export async function addUserCoal(coal: MasterCoalEntry): Promise<void> {
  await apiAddCoal(coal);
  // 服务器成功 → 立刻乐观更新 cache, 然后后台 refresh 跟上
  const all = readCache();
  all.unshift(coal); // 最新的排前
  writeCache(all);
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
  void refreshUserCoals();
}

export async function removeUserCoal(name: string): Promise<void> {
  await apiDeleteCoal(name);
  const all = readCache().filter((c) => c.name !== name);
  writeCache(all);
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
}

/** 清本地 cache - 服务器数据不动 (要清服务器请走 D1 console) */
export function clearUserCoalsCache(): void {
  localStorage.removeItem(KEY_USER_COALS);
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
}

/**
 * 一次性: 从 localStorage 把旧数据迁移到 D1.
 * 登录后调一次, 已迁移过的设备不会重复跑.
 */
export async function migrateLocalCoalsToD1(): Promise<number> {
  if (!getApiToken()) return 0;
  if (localStorage.getItem(KEY_USER_COALS_MIGRATED) === "1") return 0;
  const local = readCache();
  if (local.length === 0) {
    localStorage.setItem(KEY_USER_COALS_MIGRATED, "1");
    return 0;
  }
  try {
    const n = await apiMigrateCoals(local);
    localStorage.setItem(KEY_USER_COALS_MIGRATED, "1");
    return n;
  } catch (e) {
    console.warn("migrateLocalCoalsToD1 失败:", e);
    return 0;
  }
}

/**
 * 煤名归一化用于查重: trim + 全角空格转半角 + 大小写无关.
 * "老山兰 " / "老山兰" / "老山兰　" / "LaoShanLan" / "laoshanlan" 视为同一个名字.
 */
export function normalizeCoalName(s: string): string {
  return s.replace(/　/g, " ").trim().toLowerCase();
}

/**
 * 在已有煤种列表中找跟 candidate 同名的煤, 返回原始名字 (供 UI 提示);
 * 找不到返回 null.
 */
export function findDuplicateCoalName(
  candidate: string,
  existing: { name: string }[],
): string | null {
  const target = normalizeCoalName(candidate);
  if (!target) return null;
  const hit = existing.find((c) => normalizeCoalName(c.name) === target);
  return hit ? hit.name : null;
}

// ============================================================
// 认证 (轻量门禁, 非真正安全)
// ============================================================
//
// 账号/密码 hardcoded 在前端, 防陌生人看到, 不防恶意攻击.
// 任何人查看源码都能拿到密码, 严肃认证需要服务器, 但那违背"端算"哲学.
// 这里就是"门口提示词", 跟某些工具站的访问码同性质.

const AUTH_USER = "doudou";
const AUTH_PASS = "123456";

export function isLoggedIn(): boolean {
  return localStorage.getItem(KEY_AUTH) === "1";
}

export function tryLogin(user: string, pass: string): boolean {
  // 账号大小写无关 + 两端 trim, 密码 trim (防复制粘贴带空格/换行)
  if (user.trim().toLowerCase() === AUTH_USER && pass.trim() === AUTH_PASS) {
    localStorage.setItem(KEY_AUTH, "1");
    // 单租户共享密码: 把密码同时存为 API token, 后续请求 D1 时用
    // (服务器 secret AUTH_PASS 跟前端 hardcoded AUTH_PASS 必须一致)
    setApiToken(pass.trim());
    window.dispatchEvent(new CustomEvent("doudou:auth_changed"));
    // 后台触发: 把旧 localStorage user_coals 迁移到 D1 (只跑一次), 然后拉最新
    void migrateLocalCoalsToD1().then(() => void refreshUserCoals());
    return true;
  }
  return false;
}

export function logout(): void {
  localStorage.removeItem(KEY_AUTH);
  clearApiToken();
  window.dispatchEvent(new CustomEvent("doudou:auth_changed"));
}
