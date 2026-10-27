/**
 * 行情窄条: 炼焦煤价格指数各品种 (每个是固定规格的标准品, 服务端每晚入库),
 * 各自 最新值 + 日涨跌 + 近30日 sparkline. 估价就是按同煤种的指数对标的.
 *
 * 界面不标数据来源. 停更超过几天时在值后标出数据日期. 没数据的品种不显示,
 * 全都没有 (离线、服务异常) 时整条不渲染. 只做展示, 不进任何求解逻辑.
 * 七个品种一行放不下, 用网格换行 (手机两列、不画走势; 宽屏自动排、带走势), 见 App.css.
 * 每个品种可点开, 看完整的可交互走势 (IndexChartDialog).
 */
import { useEffect, useState } from "react";
import { IndexChartDialog } from "./IndexChartDialog";
import { changeOver, fetchMcci, staleDays } from "./mcci";
import { ChangeText, STALE_AFTER_DAYS } from "./screens/McciCard";
import type { McciSeries } from "./types";

/** sparkline 取最近这么多个交易日. */
const SPARK_POINTS = 30;

/** 窄条上的短名: 主焦三档去掉末尾的"煤" (低硫主焦煤 → 低硫主焦). */
const shortName = (typeName: string) => typeName.replace(/主焦煤$/, "主焦");

/** 内联 SVG 折线, 数据归一化到固定 viewBox, 不引图表库. */
function Sparkline({ prices, color }: { prices: number[]; color: string }) {
  const w = 56;
  const h = 18;
  const min = Math.min(...prices);
  const max = Math.max(...prices);
  const span = max - min || 1;
  const pts = prices
    .map((price, i) => {
      const x = (i / (prices.length - 1)) * w;
      const y = h - 2 - ((price - min) / span) * (h - 4);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg className="index-ticker-spark" width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden>
      <polyline points={pts} fill="none" stroke={color} strokeWidth={1.5} />
    </svg>
  );
}

function SeriesItem({ series, onOpen }: { series: McciSeries; onOpen: () => void }) {
  const points = series.points;
  const latest = points[points.length - 1];
  const pct = changeOver(points, 0);
  const rounded = pct == null ? 0 : Number(pct.toFixed(2));
  // 国内习惯: 红涨绿跌; 平盘中性灰
  const color =
    rounded === 0 ? "var(--c-text-3)" : rounded > 0 ? "var(--c-danger)" : "var(--c-success)";
  const stale = staleDays(latest.date) > STALE_AFTER_DAYS;

  return (
    <button
      type="button"
      className="index-ticker-item"
      aria-label={`${series.type_name} 最新 ${latest.price.toFixed(0)}, 查看走势`}
      onClick={onOpen}
    >
      <span>{shortName(series.type_name)}</span>
      <span style={{ fontWeight: 700, fontSize: 14, color: "var(--c-text)" }}>
        {latest.price.toFixed(0)}
      </span>
      <ChangeText pct={pct} />
      {stale && <span style={{ fontSize: 10 }}>({latest.date.slice(5)})</span>}
      {points.length >= 2 && (
        <Sparkline
          prices={points.slice(-SPARK_POINTS).map((point) => point.price)}
          color={color}
        />
      )}
    </button>
  );
}

export function IndexTicker() {
  const [series, setSeries] = useState<McciSeries[] | null>(null);
  const [open, setOpen] = useState<McciSeries | null>(null);

  useEffect(() => {
    let alive = true;
    void fetchMcci().then((next) => {
      if (alive) setSeries(next.filter((row) => row.points.length > 0));
    });
    return () => {
      alive = false;
    };
  }, []);

  if (!series || series.length === 0) return null;

  return (
    <>
      <div className="index-ticker">
        {series.map((row) => (
          <SeriesItem key={row.code} series={row} onOpen={() => setOpen(row)} />
        ))}
      </div>
      {open && <IndexChartDialog series={open} onClose={() => setOpen(null)} />}
    </>
  );
}
