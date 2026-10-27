/**
 * 焦煤现货价格指数 — 中价·新华焦煤价格指数 (CCP) 现货子指数.
 *
 * 数据经本站 Rust 服务 `/api/coal-index` 代理自 CCTD (CCTD 接口不回显 CORS 头,
 * 浏览器不能直连). 唯一用途: 一条基准报价都没有时, 拿最新点位当估价的标准品价
 * (CCP 是指数点, 2017-01 = 1000, 只是相对尺度, 不是元/吨).
 *
 * 服务不可用时返回 null, 估价就没有市场价, 今日页会说明估不出价.
 */

export interface CoalIndexPoint {
  date: string; // "2026-09-03"
  price: number; // 现货指数点位
}

const KEY_INDEX = "doudou_blend.coal_index.v1";
const TTL_MS = 6 * 60 * 60 * 1000;

function isPoint(value: unknown): value is CoalIndexPoint {
  return (
    value != null &&
    typeof value === "object" &&
    typeof (value as CoalIndexPoint).date === "string" &&
    typeof (value as CoalIndexPoint).price === "number" &&
    Number.isFinite((value as CoalIndexPoint).price) &&
    (value as CoalIndexPoint).price > 0
  );
}

/** 解析 /api/coal-index 响应 (纯函数). 返回按日期升序的有效点. */
export function parseIndexSeries(json: unknown): CoalIndexPoint[] {
  if (!Array.isArray(json)) return [];
  const points = json.filter(isPoint).map((point) => ({
    date: point.date,
    price: point.price,
  }));
  points.sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
  return points;
}

function readCache(): CoalIndexPoint[] | null {
  try {
    const raw = localStorage.getItem(KEY_INDEX);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { points?: unknown; fetchedAt?: unknown };
    if (
      typeof parsed?.fetchedAt !== "string" ||
      Date.now() - Date.parse(parsed.fetchedAt) >= TTL_MS
    ) {
      return null;
    }
    const points = parseIndexSeries(parsed.points);
    return points.length > 0 ? points : null;
  } catch {
    return null;
  }
}

async function fetchSeries(): Promise<CoalIndexPoint[] | null> {
  const cached = readCache();
  if (cached) return cached;
  try {
    const response = await fetch("/api/coal-index", { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const points = parseIndexSeries(await response.json());
    if (points.length === 0) return null;
    try {
      localStorage.setItem(
        KEY_INDEX,
        JSON.stringify({ points, fetchedAt: new Date().toISOString() }),
      );
    } catch {
      // 存不进缓存不影响本次
    }
    return points;
  } catch {
    return null;
  }
}

/**
 * 焦煤现货指数最新一期的点位 (元/吨), 当估算价的市场价 (标准品 1000 点) 用.
 * 拿不到数据返回 null.
 */
export async function fetchLatestCoalIndex(): Promise<number | null> {
  const points = await fetchSeries();
  if (points == null || points.length === 0) return null;
  return points[points.length - 1].price;
}
