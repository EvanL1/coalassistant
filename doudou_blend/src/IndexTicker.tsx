/**
 * 焦煤指数窄条: 当前价 + 涨跌幅 + 近30日 sparkline.
 * 无数据(离线且无缓存)时整条不渲染. 只做展示, 不进任何求解逻辑.
 */
import { useEffect, useState } from "react";
import { fetchJmKline, type JmKline } from "./index_quote";

/** 内联 SVG 折线, 数据归一化到固定 viewBox, 不引图表库. */
function Sparkline({ closes, color }: { closes: number[]; color: string }) {
  const w = 72;
  const h = 20;
  const min = Math.min(...closes);
  const max = Math.max(...closes);
  const span = max - min || 1;
  const pts = closes
    .map((c, i) => {
      const x = (i / (closes.length - 1)) * w;
      const y = h - 2 - ((c - min) / span) * (h - 4);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden>
      <polyline points={pts} fill="none" stroke={color} strokeWidth={1.5} />
    </svg>
  );
}

export function IndexTicker() {
  const [kline, setKline] = useState<JmKline | null>(null);

  useEffect(() => {
    let alive = true;
    fetchJmKline().then((k) => {
      if (alive) setKline(k);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (!kline || kline.points.length < 2) return null;

  const closes = kline.points.map((p) => p.close);
  const last = closes[closes.length - 1];
  const prev = closes[closes.length - 2];
  const pct = ((last - prev) / prev) * 100;
  const flat = pct === 0;
  const up = pct > 0;
  // 国内习惯: 红涨绿跌; 平盘中性灰
  const color = flat ? "var(--c-text-3)" : up ? "var(--c-danger)" : "var(--c-success)";
  const arrow = flat ? "" : up ? "▲+" : "▼";
  const lastDate = kline.points[kline.points.length - 1].date.slice(5); // MM-DD

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 2px",
        fontSize: 12,
        color: "var(--c-text-3)",
      }}
    >
      <span>焦煤JM主力</span>
      <span style={{ fontWeight: 700, fontSize: 14, color: "var(--c-text)" }}>
        {last.toFixed(1)}
      </span>
      <span style={{ color, fontWeight: 600 }}>
        {arrow}
        {Math.abs(pct).toFixed(2)}%
      </span>
      {kline.stale && <span style={{ fontSize: 10 }}>({lastDate})</span>}
      <span style={{ marginLeft: "auto", display: "flex" }}>
        <Sparkline closes={closes} color={color} />
      </span>
    </div>
  );
}
