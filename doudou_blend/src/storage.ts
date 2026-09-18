/**
 * 用户数据存储层.
 *
 * Web 端: localStorage 作为同步缓存，登录后由 PostgreSQL 状态覆盖并持续写回
 *
 * 设计:
 *   - master 是只读的(blend_kit_rs 嵌入 JSON)
 *   - 用户改的所有东西都存在这里
 *   - 重置 = 清空 localStorage 对应 key
 */

import type { Spec, MasterCoalEntry, BlendResult, MeasuredQuality } from "./types";
import { normalizeCoalName } from "./domain/coalName";
import type { AnchorQuote } from "./domain/priceDrift";
import type { CoalPenaltyOverride } from "./penalty";

export { normalizeCoalName } from "./domain/coalName";

const KEY_COAL_PREFS = "doudou_blend.coal_prefs.v1";
const KEY_CONTRACT = "doudou_blend.contract.v1";
const KEY_HISTORY = "doudou_blend.history.v1";
const KEY_USER_COALS = "doudou_blend.user_coals.v1";

/** 单个煤的用户偏好: 启用 + 价格覆盖 + 化验值覆盖 */
export interface CoalPref {
  enabled?: boolean;
  /** 读取旧存储时发现该煤偏好结构损坏，仅供解析层阻止求解。 */
  invalid_data?: boolean;
  /** 用户隐藏: true 时从煤池/求解器全部滤掉 (master 煤不能真删, 只能隐藏) */
  hidden?: boolean;
  /** 用户改后的 FOB; null = 用 master 默认 */
  fob_override?: number | null;
  /** fob 是哪天录的 (YYYY-MM-DD); 缺失时按 master updated_at 兜底 */
  fob_quoted_at?: string | null;
  /** 这个煤的报价历史, 每次改价追加一条; 锚点煤靠它构成自建价格指数 */
  fob_history?: AnchorQuote[];
  /** 标记为价格锚点: 用它的涨跌比例推算其余未更新煤的现价 */
  is_price_anchor?: boolean;
  /** 用户改后的运费; null = 用 master 默认 */
  frt_override?: number | null;
  /** 用户改过的化验项; null = 用 master 默认 */
  props_override?: Partial<Record<string, number | null>>;
  /** 该煤采购合同的保证值: 指标 → 保证值. 缺项不算扣款. */
  purchase_guarantees?: Partial<Record<string, number>>;
  /** 该煤与全局扣款模板不同的条款; 缺失 = 完全套用模板. */
  purchase_override?: CoalPenaltyOverride;
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
  /** 完整结果 (含混合后指标, 回归 X), 采集后留存; 旧记录无此字段. */
  result?: BlendResult;
  /** 回填的实测焦炭 CSR (回归 y); undefined = 未回填. */
  csr_measured?: number;
  /** 混煤实测化验回填 (信任对照 + G 修正模型样本); undefined = 未回填. */
  s_measured?: number;
  a_measured?: number;
  v_measured?: number;
  g_measured?: number;
  y_measured?: number;
  m_measured?: number;
}

/** Web 端同步到 PostgreSQL 的用户状态快照。 */
export interface UserStorageSnapshot {
  coal_prefs: CoalPrefs;
  contract: Spec[] | null;
  quantity: number;
  user_coals: MasterCoalEntry[];
}

export const USER_STORAGE_EVENTS = [
  "doudou:prefs_changed",
  "doudou:contract_changed",
  "doudou:quantity_changed",
  "doudou:user_coals_changed",
] as const;

// ============================================================
// 煤偏好
// ============================================================

export function getCoalPrefs(): CoalPrefs {
  try {
    const raw = localStorage.getItem(KEY_COAL_PREFS);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) return {};
    const prefs: CoalPrefs = {};
    for (const [name, value] of Object.entries(parsed)) {
      prefs[name] = isRecord(value)
        ? (value as CoalPref)
        : { invalid_data: true };
    }
    return prefs;
  } catch {
    return {};
  }
}

export function setCoalPref(name: string, pref: Partial<CoalPref>): void {
  const all = getCoalPrefs();
  const next: CoalPref = {
    ...all[name],
    ...pref,
    updated_at: new Date().toISOString(),
  };
  delete next.invalid_data;
  all[name] = next;
  localStorage.setItem(KEY_COAL_PREFS, JSON.stringify(all));
  // 派发自定义事件让其他组件订阅
  window.dispatchEvent(new CustomEvent("doudou:prefs_changed"));
}

export function getCoalPref(name: string): CoalPref | null {
  return getCoalPrefs()[name] ?? null;
}

/** 本地日期 YYYY-MM-DD. 不能用 toISOString(那是 UTC), 国内凌晨会差一天. */
function todayLocal(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

const MAX_QUOTE_HISTORY = 60;

/**
 * 录一次价: 写 fob 覆盖 + 录价日, 并追加到该煤的报价历史.
 *
 * `seed` 是改价之前的口径(通常是 master 的 {updated_at, fob}), 只在历史为空时
 * 用来补起点 —— 没有起点就只有一个点, 算不出涨跌比例, 锚点也就失效了.
 */
export function recordCoalQuote(
  name: string,
  fob: number,
  seed?: AnchorQuote | null,
): void {
  const date = todayLocal();
  const previous = getCoalPref(name);
  const history: AnchorQuote[] = Array.isArray(previous?.fob_history)
    ? [...previous.fob_history]
    : [];
  if (history.length === 0 && seed != null && seed.date < date) {
    history.push({ date: seed.date, fob: seed.fob });
  }
  const sameDay = history.findIndex((quote) => quote?.date === date);
  if (sameDay >= 0) {
    history[sameDay] = { date, fob };
  } else {
    history.push({ date, fob });
  }
  history.sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
  setCoalPref(name, {
    fob_override: fob,
    fob_quoted_at: date,
    fob_history: history.slice(-MAX_QUOTE_HISTORY),
  });
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

export function enableAllCoals(names: string[]): void {
  const all = getCoalPrefs();
  const now = new Date().toISOString();
  for (const name of names) {
    all[name] = { ...all[name], enabled: true, updated_at: now } as CoalPref;
  }
  localStorage.setItem(KEY_COAL_PREFS, JSON.stringify(all));
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
// 采购总量 (只缩放实物订单, 不影响配方/单价/可行性)
// ============================================================

const KEY_QUANTITY = "doudou_blend.quantity.v1";
const DEFAULT_QUANTITY = 3700;

/** 采购总吨数. 缺失/非法时回退默认 3700. */
export function getQuantity(): number {
  try {
    const raw = localStorage.getItem(KEY_QUANTITY);
    if (raw == null) return DEFAULT_QUANTITY;
    const n = Number(raw);
    return Number.isFinite(n) && n > 0 ? n : DEFAULT_QUANTITY;
  } catch {
    return DEFAULT_QUANTITY;
  }
}

export function setQuantity(n: number): void {
  localStorage.setItem(KEY_QUANTITY, String(n));
  window.dispatchEvent(new CustomEvent("doudou:quantity_changed"));
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

/** 回填某条历史的混煤实测化验 (部分字段, 只更新提供的项). id 不存在则静默忽略. */
export function setMeasuredQualityLocal(id: string, m: MeasuredQuality): void {
  const all = getHistory();
  if (!all.some((e) => e.id === id)) return;
  const updated = all.map((e) => {
    if (e.id !== id) return e;
    const next = { ...e };
    if (m.s != null) next.s_measured = m.s;
    if (m.a != null) next.a_measured = m.a;
    if (m.v != null) next.v_measured = m.v;
    if (m.g != null) next.g_measured = m.g;
    if (m.y != null) next.y_measured = m.y;
    if (m.m != null) next.m_measured = m.m;
    if (m.csr != null) next.csr_measured = m.csr;
    return next;
  });
  localStorage.setItem(KEY_HISTORY, JSON.stringify(updated));
  window.dispatchEvent(new CustomEvent("doudou:history_changed"));
}

// ============================================================
// 用户新增的煤种
// ============================================================
//
// Master 煤种是只读 (嵌入核心 crate), 用户新增的煤暂存这里.
// 新增时仅录煤名/产地/煤类, 化验值后续在 CoalEditor 里补 (status=draft).
// 补全价格并启用后, 与 Master 煤走同一套解析规则参与求解.

export function getUserCoals(): MasterCoalEntry[] {
  try {
    const raw = localStorage.getItem(KEY_USER_COALS);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter(isMasterCoalEntry)
      : [];
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

export function removeUserCoal(
  name: string,
  preservePref: boolean = false,
): void {
  const all = getUserCoals().filter((c) => c.name !== name);
  localStorage.setItem(KEY_USER_COALS, JSON.stringify(all));
  if (!preservePref) {
    clearCoalPref(name);
  }
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
}

export function clearUserCoals(masterCoalNames: readonly string[]): void {
  const masterNames = new Set(masterCoalNames.map(normalizeCoalName));
  const userNames = new Set(
    getUserCoals()
      .filter((coal) => !masterNames.has(normalizeCoalName(coal.name)))
      .map((coal) => coal.name),
  );
  const prefs = getCoalPrefs();
  for (const name of userNames) {
    delete prefs[name];
  }
  localStorage.removeItem(KEY_USER_COALS);
  if (Object.keys(prefs).length > 0) {
    localStorage.setItem(KEY_COAL_PREFS, JSON.stringify(prefs));
  } else {
    localStorage.removeItem(KEY_COAL_PREFS);
  }
  window.dispatchEvent(new CustomEvent("doudou:prefs_changed"));
  window.dispatchEvent(new CustomEvent("doudou:user_coals_changed"));
}

/** 读取当前本地缓存，供 Web 首次导入和持续同步使用。 */
export function getUserStorageSnapshot(): UserStorageSnapshot {
  return {
    coal_prefs: getCoalPrefs(),
    contract: getUserContract(),
    quantity: getQuantity(),
    user_coals: getUserCoals(),
  };
}

/** 用服务端快照原子替换本地缓存；调用方应在启动同步监听前执行。 */
export function replaceUserStorageSnapshot(snapshot: UserStorageSnapshot): void {
  if (Object.keys(snapshot.coal_prefs).length > 0) {
    localStorage.setItem(KEY_COAL_PREFS, JSON.stringify(snapshot.coal_prefs));
  } else {
    localStorage.removeItem(KEY_COAL_PREFS);
  }

  if (snapshot.contract) {
    localStorage.setItem(KEY_CONTRACT, JSON.stringify(snapshot.contract));
  } else {
    localStorage.removeItem(KEY_CONTRACT);
  }

  localStorage.setItem(KEY_QUANTITY, String(snapshot.quantity));

  if (snapshot.user_coals.length > 0) {
    localStorage.setItem(KEY_USER_COALS, JSON.stringify(snapshot.user_coals));
  } else {
    localStorage.removeItem(KEY_USER_COALS);
  }

  for (const eventName of USER_STORAGE_EVENTS) {
    window.dispatchEvent(new CustomEvent(eventName));
  }
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return (
    value != null &&
    typeof value === "object" &&
    !Array.isArray(value)
  );
}

const COAL_STATUSES = new Set([
  "verified",
  "active",
  "draft",
  "incomplete",
  "archived",
]);

function isMasterCoalEntry(value: unknown): value is MasterCoalEntry {
  if (!isRecord(value)) return false;
  return (
    typeof value.name === "string" &&
    normalizeCoalName(value.name).length > 0 &&
    COAL_STATUSES.has(String(value.status)) &&
    isRecord(value.props)
  );
}
