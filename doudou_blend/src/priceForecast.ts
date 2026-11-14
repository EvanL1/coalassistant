/**
 * 一周后的预测价: 服务端按钢铁网指数的近期走势推算 (blend_kit_server/src/forecast.rs),
 * 煤的预测价是按预测的指数把共享煤库重估一遍. 顺势外推, 拐点会滞后 —— 只作参考展示, 不进求解.
 */
import { staleDays } from "./mcci";

export interface PriceForecast {
  /** 预测的是哪天 (YYYY-MM-DD). */
  date: string;
  price: number;
  /** 现在的价, 涨跌相对它算. */
  base: number;
}

/** 预测的那天还没到才算数: 快照或指数停更后, 旧预测不再显示. */
export function isAhead(date: string, now: Date = new Date()): boolean {
  return staleDays(date, now) < 0;
}

/** 涨跌记号 (按元取整): ▲12 / ▼8 / 持平. */
export function changeMark(change: number): string {
  const rounded = Math.round(Math.abs(change));
  if (rounded === 0) return "持平";
  return `${change > 0 ? "▲" : "▼"}${rounded}`;
}

export const yuan = (price: number) => Math.round(price).toLocaleString("zh-CN");
