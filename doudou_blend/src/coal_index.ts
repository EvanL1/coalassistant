/**
 * 焦煤现货价格指数 — 中价·新华焦煤价格指数 (CCP) 现货子指数.
 *
 * 数据经本站 Rust 服务 `/api/coal-index` 代理自 CCTD (CCTD 接口不回显 CORS 头,
 * 浏览器不能直连). 用途只是今日屏"若随现货指数变动"的参考估算, 不进求解.
 *
 * 相比原先的期货主连方案: 现货更贴合实际采购、口径是国家基准指数、且**没有换月
 * 跳空**(现货指数不拼合约), 因此不再需要任何复权逻辑. 仍与用户中高硫山西煤有
 * 基差, 定位仍是参考锚而非成交价.
 *
 * 服务不可用时返回 null, 参考估算不显示 (报价时效警告是纯前端, 不受影响).
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

/** 序列里 date 当天或之前最近一点的值; date 早于全部数据时返回 null. */
export function indexPriceOn(
  points: readonly CoalIndexPoint[],
  date: string,
): number | null {
  let best: CoalIndexPoint | null = null;
  for (const point of points) {
    if (point.date > date) continue;
    if (best == null || point.date > best.date) best = point;
  }
  return best?.price ?? null;
}

/**
 * 现货指数自 `date` 起的涨跌比例. 现货无换月, 直接最新价 ÷ 基准价.
 * 定不出基准(date 早于全部数据)或无最新点时返回 null.
 */
export function indexRatioSince(
  points: readonly CoalIndexPoint[],
  date: string,
): number | null {
  if (points.length === 0) return null;
  const base = indexPriceOn(points, date);
  const latest = points[points.length - 1].price;
  if (base == null || !(base > 0) || !(latest > 0)) return null;
  return latest / base;
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
 * 焦煤现货指数自 `date` 起的涨跌比例, 供今日屏参考估算.
 * 拿不到数据(源站不可用)返回 null, 界面就不显示这条参考.
 */
export async function fetchCoalIndexRatioSince(
  date: string,
): Promise<number | null> {
  const points = await fetchSeries();
  if (points == null) return null;
  return indexRatioSince(points, date);
}
