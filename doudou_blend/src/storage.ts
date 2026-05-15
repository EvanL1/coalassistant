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

const KEY_COAL_PREFS = "doudou_blend.coal_prefs.v1";
const KEY_CONTRACT = "doudou_blend.contract.v1";
const KEY_HISTORY = "doudou_blend.history.v1";
const KEY_AUTH = "doudou_blend.auth.v1";
const KEY_USER_COALS = "doudou_blend.user_coals.v1";

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
// 用户新增的煤种
// ============================================================
//
// master 73 种煤是只读 (嵌入 WASM), 用户新增的煤暂存这里.
// 新增时仅录煤名/产地/煤类, 化验值后续在 CoalEditor 里补 (status=draft).
// 注: 当前不参与求解, 等用户在 CoalEditor 补全化验值并启用后, 后续接求解器再说.

export function getUserCoals(): MasterCoalEntry[] {
  try {
    const raw = localStorage.getItem(KEY_USER_COALS);
    return raw ? (JSON.parse(raw) as MasterCoalEntry[]) : [];
  } catch {
    return [];
  }
}

export function addUserCoal(coal: MasterCoalEntry): void {
  const all = getUserCoals();
  all.push(coal);
  localStorage.setItem(KEY_USER_COALS, JSON.stringify(all));
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
}

export function removeUserCoal(name: string): void {
  const all = getUserCoals().filter((c) => c.name !== name);
  localStorage.setItem(KEY_USER_COALS, JSON.stringify(all));
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
}

export function clearUserCoals(): void {
  localStorage.removeItem(KEY_USER_COALS);
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
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
    window.dispatchEvent(new CustomEvent("doudou:auth_changed"));
    return true;
  }
  return false;
}

export function logout(): void {
  localStorage.removeItem(KEY_AUTH);
  window.dispatchEvent(new CustomEvent("doudou:auth_changed"));
}
