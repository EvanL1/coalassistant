/**
 * 询价精算的草稿: 还没点「精算」就切走页签时, 回来能接着填.
 *
 * 只存 sessionStorage (关掉浏览器标签页就没了), 点了「精算」或「返回今日」即清掉 ——
 * 询价不跨次沿用, 完成一次精算后下次进来仍是空的.
 */
import type { BlendResult, IndicatorKey } from "../types";

const KEY_QUOTE_DRAFT = "doudou_blend.quote_draft.v1";

export interface QuoteDraftRow {
  name: string;
  fob: string;
  frt: string;
  props: Partial<Record<IndicatorKey, string>>;
}

export interface QuoteDraft {
  /** 进入精算页那一刻的今日估价结果 (默认煤、今日估价都从它来). */
  todayResult: BlendResult;
  /** 存档批次: 草稿恢复后接着用同一批, 服务端按批覆盖. */
  batchId: string;
  /** null = 还没动过, 按今日配方列默认煤. */
  rows: QuoteDraftRow[] | null;
}

function store(): Storage | null {
  try {
    return typeof sessionStorage === "undefined" ? null : sessionStorage;
  } catch {
    return null;
  }
}

function isText(value: unknown): value is string {
  return typeof value === "string";
}

function isRow(value: unknown): value is QuoteDraftRow {
  if (value == null || typeof value !== "object") return false;
  const row = value as Partial<QuoteDraftRow>;
  return (
    isText(row.name) &&
    isText(row.fob) &&
    isText(row.frt) &&
    row.props != null &&
    typeof row.props === "object" &&
    Object.values(row.props).every(isText)
  );
}

export function loadQuoteDraft(): QuoteDraft | null {
  try {
    const raw = store()?.getItem(KEY_QUOTE_DRAFT);
    if (!raw) return null;
    const draft = JSON.parse(raw) as Partial<QuoteDraft>;
    if (
      draft.todayResult == null ||
      typeof draft.todayResult !== "object" ||
      !Array.isArray(draft.todayResult.orders) ||
      !isText(draft.batchId) ||
      (draft.rows !== null && !(Array.isArray(draft.rows) && draft.rows.every(isRow)))
    ) {
      return null;
    }
    return draft as QuoteDraft;
  } catch {
    return null;
  }
}

export function saveQuoteDraft(draft: QuoteDraft): void {
  try {
    store()?.setItem(KEY_QUOTE_DRAFT, JSON.stringify(draft));
  } catch {
    // 存不进去 (容量、隐私模式) 只是回来时不能接着填
  }
}

export function clearQuoteDraft(): void {
  try {
    store()?.removeItem(KEY_QUOTE_DRAFT);
  } catch {
    // 同上
  }
}
