/**
 * 钢铁网国产炼焦煤价格指数 (MCCI) 的数据层: 服务端每晚 17:30 后拉一次存库,
 * /api/mcci 只读库. 每条序列是一个固定规格的标准品, 估价按同煤种的序列对标.
 * 只做展示, 求解用的基准由 /api/benchmarks 给.
 */
import type { McciSeries, McciPoint } from "./types";

/** 指标展示顺序与写法 (M 按行业习惯写 Mt). */
const SPEC_ORDER: [key: string, label: string][] = [
  ["A", "A"],
  ["V", "V"],
  ["S", "S"],
  ["G", "G"],
  ["Y", "Y"],
  ["M", "Mt"],
];

const DAY_MS = 24 * 60 * 60 * 1000;
const BEIJING_OFFSET_MS = 8 * 60 * 60 * 1000;

/** 指数总览. 取不到 (离线、服务异常) 返回空表, 卡片不显示. */
export async function fetchMcci(): Promise<McciSeries[]> {
  try {
    const response = await fetch("/api/mcci", { credentials: "same-origin", cache: "no-store" });
    if (!response.ok) return [];
    const result = (await response.json()) as { series?: McciSeries[] };
    return result.series ?? [];
  } catch {
    return [];
  }
}

function dayNumber(date: string): number {
  return Date.parse(`${date}T00:00:00Z`) / DAY_MS;
}

/** 涨跌幅 (%): days=0 比上一个交易日; 否则比 days 天前 (含) 最近的那天, 但不早于再往前 days 天
 * (中间断了数据时不拿几周前的点冒充"周涨跌"). 不够比返回 null. */
export function changeOver(points: McciPoint[], days: number): number | null {
  const last = points[points.length - 1];
  if (!last) return null;
  const cutoff = dayNumber(last.date) - days;
  const base =
    days === 0
      ? points[points.length - 2]
      : [...points]
          .reverse()
          .find((point) => dayNumber(point.date) <= cutoff && dayNumber(point.date) > cutoff - days);
  if (!base || base.price <= 0) return null;
  return ((last.price - base.price) / base.price) * 100;
}

/** 最新一天离今天 (北京时间) 几个日历天. */
export function staleDays(latest: string, now: Date = new Date()): number {
  const today = Math.floor((now.getTime() + BEIJING_OFFSET_MS) / DAY_MS);
  return today - dayNumber(latest);
}

/** 只留最新一天往前 days 个日历天 (按最新数据日算, 不按今天, 停更时不会一下子变空). */
export function lastDays(points: McciPoint[], days: number): McciPoint[] {
  const last = points[points.length - 1];
  if (!last) return [];
  const since = dayNumber(last.date) - days;
  return points.filter((point) => dayNumber(point.date) > since);
}

/** 只留最新一天往前一年. */
export function lastYear(points: McciPoint[]): McciPoint[] {
  return lastDays(points, 365);
}

/** 标准品指标, 如 "A10 V30 S1.8 G90 Mt10". */
export function specText(props: Record<string, number>): string {
  return SPEC_ORDER.filter(([key]) => props[key] != null)
    .map(([key, label]) => `${label}${props[key]}`)
    .join(" ");
}
