import type { ResolvedCoal } from "./domain/resolvedCoal";
import type { Coal, CoalPriceHistory, CoalPriceSnapshot, CoalValue, PriceResult } from "./types";

export type CardPrice =
  | { kind: "snapshot"; price: number; change: number | null; missing: string[] }
  /** modified: 用户改过化验值或自己新增; 否则只是快照里没有这种煤 (如还没生成快照). */
  | { kind: "live"; price: number; missing: string[]; modified: boolean }
  | { kind: "unavailable" };

export async function fetchCoalPrices(): Promise<CoalPriceSnapshot | null> {
  try {
    const response = await fetch("/api/coal-prices", {
      credentials: "same-origin", cache: "no-store",
    });
    if (!response.ok) return null;
    const result = (await response.json()) as CoalPriceSnapshot;
    return result.ok && Array.isArray(result.prices) ? result : null;
  } catch {
    return null;
  }
}

export async function fetchCoalPriceHistory(name: string): Promise<CoalPriceHistory | null> {
  try {
    const response = await fetch(`/api/coal-prices/history?name=${encodeURIComponent(name)}`, {
      credentials: "same-origin", cache: "no-store",
    });
    if (!response.ok) return null;
    const result = (await response.json()) as CoalPriceHistory;
    return result?.ok === true && result.name === name && Array.isArray(result.points)
      && result.points.every((point) => point
        && typeof point.day === "string" && /^\d{4}-\d{2}-\d{2}$/.test(point.day)
        && Number.isFinite(point.price)
        && (point.benchmark === null || typeof point.benchmark === "string")
        && (point.source === "live" || point.source === "backfill"))
      ? result : null;
  } catch {
    return null;
  }
}

/** 历史按共享煤库记; 用户自己新增的煤 (哪怕同名) 没有自己的历史. */
export function hasPriceHistory(coal: ResolvedCoal, snapshot: CoalPriceSnapshot | null): boolean {
  return coal.origin !== "user" && !!snapshot?.prices.some((entry) => entry.name === coal.name);
}

export async function estimateLive(coals: Coal[]): Promise<Map<string, CoalValue> | null> {
  if (coals.length === 0) return new Map();
  try {
    const response = await fetch("/api/price", {
      method: "POST", credentials: "same-origin",
      headers: { "Content-Type": "application/json" }, body: JSON.stringify({ coals }),
    });
    if (!response.ok) return null;
    const result = (await response.json()) as PriceResult;
    if (!result.ok || !Array.isArray(result.coal_values)) return null;
    return new Map(result.coal_values.map((value) => [value.name, value]));
  } catch {
    return null;
  }
}

export function isModified(coal: ResolvedCoal): boolean {
  return coal.origin === "user" || Object.keys(coal.overriddenProps).length > 0;
}

function needsLive(coal: ResolvedCoal, snapshot: CoalPriceSnapshot | null): boolean {
  return isModified(coal) || !snapshot?.prices.some((entry) => entry.name === coal.name);
}

function canEstimate(coal: ResolvedCoal): boolean {
  return !coal.invalidOverride && coal.readiness !== "duplicate_name";
}

/** 卡片给停用煤也估价, 不能复用只放行 ready 的 toEstimateCoal. */
export function coalsToEstimate(coals: ResolvedCoal[], snapshot: CoalPriceSnapshot | null): Coal[] {
  return coals.filter((coal) => canEstimate(coal) && needsLive(coal, snapshot))
    .map((coal) => ({
      name: coal.name, props: { ...coal.props }, fob: 0, frt: 0,
      coal_type: coal.coal_type, form: coal.form, region: coal.region,
    }));
}

export function cardPrice(
  coal: ResolvedCoal,
  snapshot: CoalPriceSnapshot | null,
  live: Map<string, CoalValue> | null,
): CardPrice {
  if (!canEstimate(coal)) return { kind: "unavailable" };
  if (needsLive(coal, snapshot)) {
    const value = live?.get(coal.name);
    return value
      ? { kind: "live", price: value.fair, missing: value.missing, modified: isModified(coal) }
      : { kind: "unavailable" };
  }
  const entry = snapshot!.prices.find((entry) => entry.name === coal.name)!;
  return {
    kind: "snapshot", price: entry.price, missing: entry.missing,
    change: entry.prev_price == null ? null : entry.price - entry.prev_price,
  };
}
