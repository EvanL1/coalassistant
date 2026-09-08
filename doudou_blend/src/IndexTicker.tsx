/**
 * 期货行情窄条: 焦煤 / 焦炭主力连续, 各自 当前价 + 涨跌幅 + 近30日 sparkline.
 *
 * 涨跌幅直接用东财的交易所口径字段(相对昨结算价), 不自己拿收盘价相减 —— 见
 * index_quote.ts 顶部说明. 无数据(离线且无缓存)时整条不渲染.
 * 只做展示, 不进任何求解逻辑.
 */
import { useEffect, useState } from "react";
import { fetchQuotes, SPARK_POINTS, type Quote } from "./index_quote";

/** 内联 SVG 折线, 数据归一化到固定 viewBox, 不引图表库. */
function Sparkline({ closes, color }: { closes: number[]; color: string }) {
  const w = 56;
  const h = 18;
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

function QuoteItem({ quote }: { quote: Quote }) {
  const flat = quote.pct === 0;
  const up = quote.pct > 0;
  // 国内习惯: 红涨绿跌; 平盘中性灰
  const color = flat
    ? "var(--c-text-3)"
    : up
    ? "var(--c-danger)"
    : "var(--c-success)";
  const arrow = flat ? "" : up ? "▲+" : "▼";

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        whiteSpace: "nowrap",
      }}
    >
      <span>{quote.label}</span>
      <span style={{ fontWeight: 700, fontSize: 14, color: "var(--c-text)" }}>
        {quote.price.toFixed(1)}
      </span>
      <span style={{ color, fontWeight: 600 }}>
        {arrow}
        {Math.abs(quote.pct).toFixed(2)}%
      </span>
      {quote.stale && quote.lastDate && (
        <span style={{ fontSize: 10 }}>({quote.lastDate})</span>
      )}
      {quote.points.length >= 2 && (
        <Sparkline
          closes={quote.points.slice(-SPARK_POINTS).map((point) => point.close)}
          color={color}
        />
      )}
    </div>
  );
}

export function IndexTicker() {
  const [quotes, setQuotes] = useState<Quote[] | null>(null);

  useEffect(() => {
    let alive = true;
    fetchQuotes().then((next) => {
      if (alive) setQuotes(next);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (!quotes || quotes.length === 0) return null;

  return (
    <div
      className="index-ticker"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        padding: "6px 2px",
        fontSize: 12,
        color: "var(--c-text-3)",
        overflowX: "auto",
      }}
    >
      {quotes.map((quote) => (
        <QuoteItem key={quote.secid} quote={quote} />
      ))}
    </div>
  );
}
