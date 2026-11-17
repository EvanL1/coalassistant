/** 涨跌幅展示: 行情窄条 (IndexTicker) 与指数卡片 (McciCard) 共用. */
import type { ReactElement } from "react";

/** 超过这么多个日历天没新值就提示 (周末 + 一天假期以内不算). */
export const STALE_AFTER_DAYS = 4;

/** 国内习惯: 红涨绿跌, 平盘中性. rounded 是保留两位小数后的涨跌幅. */
export function changeColor(rounded: number): string {
  return rounded === 0 ? "var(--c-text-3)" : rounded > 0 ? "var(--c-danger)" : "var(--c-success)";
}

export function ChangeText({ pct }: { pct: number | null }): ReactElement {
  if (pct == null) return <span style={{ fontSize: 11, color: "var(--c-text-3)" }}>—</span>;
  const rounded = Number(pct.toFixed(2));
  const arrow = rounded === 0 ? "" : rounded > 0 ? "▲+" : "▼";
  return (
    <span style={{ color: changeColor(rounded), fontWeight: 600 }}>
      {arrow}
      {Math.abs(rounded).toFixed(2)}%
    </span>
  );
}
