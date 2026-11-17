import type { ArchivedQuote, ArchiveQuotesResponse, IndicatorKey, LatestQuote, LatestQuotesResponse } from "./types";
import { ApiError, apiJson, apiJsonOrNull, genericFailure } from "./http";

/** 本次询价: 出厂价 + 运费 + 这批实际用的指标, 不写入煤池. */
export interface QuoteItemInput {
  name: string;
  fob: number;
  frt: number;
  coal_type?: string;
  form?: string;
  region?: string;
  province?: string;
  city?: string;
  county?: string;
  props: Partial<Record<IndicatorKey, number | null>>;
  changed: IndicatorKey[];
}

export async function archiveQuotes(batchId: string, items: QuoteItemInput[]): Promise<ArchivedQuote[]> {
  const failed = (status: number) => `询价存档失败 (HTTP ${status})`;
  const result = await apiJson<(ArchiveQuotesResponse & { reason?: string }) | null>("/api/quotes", {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ batch_id: batchId, items }),
  }).catch((error: unknown) => {
    // 服务端没给原因时, 说清失败的是存档这一步.
    if (error instanceof ApiError && error.message === genericFailure(error.status)) {
      throw new Error(failed(error.status));
    }
    throw error;
  });
  if (result?.ok !== true) throw new Error(result?.reason || failed(200));
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
  const result = await apiJsonOrNull<LatestQuotesResponse>("/api/quotes/latest", { cache: "no-store" });
  if (result?.ok !== true || !Array.isArray(result.quotes) || !result.quotes.every((quote) => quote
    && typeof quote.name === "string" && Number.isFinite(quote.fob) && Number.isFinite(quote.frt)
    && typeof quote.quoted_on === "string" && /^\d{4}-\d{2}-\d{2}$/.test(quote.quoted_on))) return new Map();
  return new Map(result.quotes.map((quote) => [quote.name, quote]));
}
