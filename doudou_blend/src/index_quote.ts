/**
 * 焦煤指数行情 (大商所 JM 主力连续) — 纯前端数据层.
 *
 * 数据源: 东方财富日K接口, 响应带 CORS 头, 浏览器 / Tauri webview 均可直接 fetch.
 * 缓存: localStorage, 10 分钟内不重复请求; fetch 失败退回缓存并标 stale.
 * 设计: docs/superpowers/specs/2026-07-08-price-index-widget-design.md
 */

export interface KlinePoint {
  date: string; // "2026-07-08"
  close: number; // 收盘价 元/吨
}

export interface JmKline {
  points: KlinePoint[];
  fetchedAt: string; // 抓取时刻 ISO
  stale: boolean; // true = 本次抓取失败, 展示的是缓存旧值
}

const KEY_KLINE = "doudou_blend.jm_kline.v1";
const TTL_MS = 10 * 60 * 1000;
const POINTS = 30;

const API_URL =
  "https://push2his.eastmoney.com/api/qt/stock/kline/get" +
  "?secid=114.jmm&klt=101&fqt=0" +
  "&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57" +
  "&end=20500101&lmt=60";

/** 解析东财响应 (纯函数, 无 IO). klines 每条 "日期,开,收,高,低,量,额". */
export function parseEastmoneyKlines(json: unknown, take: number): KlinePoint[] {
  const klines = (json as { data?: { klines?: unknown } })?.data?.klines;
  if (!Array.isArray(klines)) return [];
  const points: KlinePoint[] = [];
  for (const line of klines) {
    if (typeof line !== "string") continue;
    const parts = line.split(",");
    const close = Number(parts[2]);
    if (parts[0] && Number.isFinite(close)) {
      points.push({ date: parts[0], close });
    }
  }
  return points.slice(-take);
}

function readCache(): JmKline | null {
  try {
    const raw = localStorage.getItem(KEY_KLINE);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as JmKline;
    if (!Array.isArray(parsed.points) || parsed.points.length === 0) return null;
    return parsed;
  } catch {
    return null;
  }
}

/** 取近 30 日焦煤主连日K. 缓存新鲜直接返回; 失败退缓存(stale); 全无 → null. */
export async function fetchJmKline(): Promise<JmKline | null> {
  const cached = readCache();
  if (cached && Date.now() - Date.parse(cached.fetchedAt) < TTL_MS) {
    return cached;
  }
  try {
    const resp = await fetch(API_URL);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const points = parseEastmoneyKlines(await resp.json(), POINTS);
    if (points.length === 0) throw new Error("empty klines");
    const fresh: JmKline = {
      points,
      fetchedAt: new Date().toISOString(),
      stale: false,
    };
    try {
      localStorage.setItem(KEY_KLINE, JSON.stringify(fresh));
    } catch {
      // 存不进缓存不影响本次展示
    }
    return fresh;
  } catch {
    return cached ? { ...cached, stale: true } : null;
  }
}
