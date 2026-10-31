import type { ArchivedQuote, ArchiveQuotesResponse, IndicatorKey, LatestQuote, LatestQuotesResponse } from "./types";

/** 本次询价: 出厂价 + 运费 + 这批实际用的指标, 不写入煤池. */
export interface QuoteItemInput {
  name: string;
  fob: number;
  frt: number;
  coal_type?: string;
  form?: string;
  region?: string;
  props: Partial<Record<IndicatorKey, number | null>>;
  changed: IndicatorKey[];
}

export async function archiveQuotes(batchId: string, items: QuoteItemInput[]): Promise<ArchivedQuote[]> {
  const response = await fetch("/api/quotes", {
    method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ batch_id: batchId, items }),
  });
  const result = await response.json().catch(() => null) as (ArchiveQuotesResponse & { reason?: string }) | null;
  if (!response.ok || result?.ok !== true) {
    throw new Error(result?.reason || `询价存档失败 (HTTP ${response.status})`);
  }
  if (!Array.isArray(result.items) || !result.items.every((item) => item
    && typeof item.name === "string" && Number.isFinite(item.fob)
    && [item.estimate, item.gap, item.ratio].every((value) => value === null || Number.isFinite(value))
    && (item.benchmark === null || typeof item.benchmark === "string"))) {
    throw new Error("询价存档响应格式不对");
  }
  return result.items;
}

/** 上次询价只作占位提示, 失败不影响本次填价. */
export async function fetchLatestQuotes(): Promise<Map<string, LatestQuote>> {
  try {
    const response = await fetch("/api/quotes/latest", { credentials: "same-origin", cache: "no-store" });
    if (!response.ok) return new Map();
    const result = await response.json() as LatestQuotesResponse;
    if (result?.ok !== true || !Array.isArray(result.quotes) || !result.quotes.every((quote) => quote
      && typeof quote.name === "string" && Number.isFinite(quote.fob) && Number.isFinite(quote.frt)
      && typeof quote.quoted_on === "string" && /^\d{4}-\d{2}-\d{2}$/.test(quote.quoted_on))) return new Map();
    return new Map(result.quotes.map((quote) => [quote.name, quote]));
  } catch {
    return new Map();
  }
}
