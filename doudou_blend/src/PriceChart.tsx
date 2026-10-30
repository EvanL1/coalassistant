/**
 * 指数与煤价弹层共用的走势图: 十字线 + tooltip, 键盘 ←/→/Home/End 逐日读值.
 * 单条序列, 不放图例 (标题就是名字); 网格与坐标轴用边框/弱文字色, 线用主色.
 * 图宽跟容器走 (手机全宽), 高度固定. 可按来源分虚实段, 在基准变更处加竖标.
 */
import { useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";

const CHART_H = 220;
/** 测不到容器宽度时 (首帧、测试环境) 用的宽度. */
const FALLBACK_W = 600;
const PAD = { top: 12, right: 12, bottom: 24, left: 44 };
const Y_TICKS = 4;
const X_LABELS = 5;

/** 取整到 1/2/5×10ⁿ 的刻度, 让纵轴读数是整数. */
function niceTicks(min: number, max: number, count: number): number[] {
  const span = max - min || Math.abs(max) || 1;
  const raw = span / count;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((k) => k * magnitude).find((s) => s >= raw) ?? raw;
  const start = Math.floor(min / step) * step;
  const end = Math.ceil(max / step) * step;
  const ticks: number[] = [];
  for (let value = start; value <= end + step / 2; value += step) ticks.push(value);
  return ticks.length >= 2 ? ticks : [start, start + step];
}

/** 等距取 count 个下标 (含首尾), 作横轴日期标签. */
function labelIndexes(length: number, count: number): number[] {
  if (length <= count) return Array.from({ length }, (_, i) => i);
  return Array.from({ length: count }, (_, k) => Math.round((k * (length - 1)) / (count - 1)));
}

function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(FALLBACK_W);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => {
      if (node.clientWidth > 0) setWidth(node.clientWidth);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

export interface ChartPoint {
  date: string;
  price: number;
  /** 画成虚线 (如回算的估价); 不传为实线. */
  dashed?: boolean;
  /** tooltip 里价格、日期之后多显示的几行. */
  notes?: string[];
}

/** 相邻同为虚线 (或同为实线) 的点连成一段; 段与段共用交界点, 线不断开. */
function runs(points: ChartPoint[]): { dashed: boolean; from: number; to: number }[] {
  const result: { dashed: boolean; from: number; to: number }[] = [];
  let from = 0;
  for (let i = 1; i < points.length; i++) {
    if (!!points[i].dashed !== !!points[from].dashed) {
      result.push({ dashed: !!points[from].dashed, from, to: i });
      from = i;
    }
  }
  result.push({ dashed: !!points[from].dashed, from, to: points.length - 1 });
  return result;
}

export function PriceChart({
  label,
  points,
  markers = [],
}: {
  label: string;
  points: ChartPoint[];
  /** 要在横轴上方画竖标的点下标 (如对标基准变了的那天). */
  markers?: number[];
}) {
  const [wrapRef, width] = useWidth<HTMLDivElement>();
  const [active, setActive] = useState<number | null>(null);
  const prices = points.map((point) => point.price);
  const ticks = niceTicks(Math.min(...prices), Math.max(...prices), Y_TICKS);
  const low = ticks[0];
  const high = ticks[ticks.length - 1];
  const innerW = Math.max(width - PAD.left - PAD.right, 1);
  const innerH = CHART_H - PAD.top - PAD.bottom;
  const last = points.length - 1;
  // 按交易日等距排 (周末、节假日不留空档), 与行情软件一致.
  const x = (i: number) => PAD.left + (last > 0 ? (i / last) * innerW : innerW / 2);
  const y = (price: number) => PAD.top + (1 - (price - low) / (high - low || 1)) * innerH;
  const hovered = active != null ? points[active] : null;

  const onPointerMove = (event: PointerEvent<SVGSVGElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const scale = rect.width > 0 ? width / rect.width : 1;
    const ratio = ((event.clientX - rect.left) * scale - PAD.left) / innerW;
    setActive(Math.round(Math.min(Math.max(ratio, 0), 1) * last));
  };
  const onKeyDown = (event: KeyboardEvent<SVGSVGElement>) => {
    const current = active ?? last;
    const next =
      event.key === "ArrowLeft"
        ? current - 1
        : event.key === "ArrowRight"
        ? current + 1
        : event.key === "Home"
        ? 0
        : event.key === "End"
        ? last
        : null;
    if (next == null) return;
    event.preventDefault();
    setActive(Math.min(Math.max(next, 0), last));
  };

  return (
    <div ref={wrapRef} className="index-chart">
      <svg
        role="img"
        tabIndex={0}
        aria-label={`${label}走势, ${points.length} 个交易日, 低 ${Math.min(...prices).toFixed(0)} 高 ${Math.max(...prices).toFixed(0)}. 左右方向键逐日查看`}
        width={width}
        height={CHART_H}
        viewBox={`0 0 ${width} ${CHART_H}`}
        onPointerMove={onPointerMove}
        onPointerLeave={() => setActive(null)}
        onKeyDown={onKeyDown}
        onBlur={() => setActive(null)}
      >
        {ticks.map((tick) => (
          <g key={tick}>
            <line
              x1={PAD.left}
              x2={PAD.left + innerW}
              y1={y(tick)}
              y2={y(tick)}
              stroke="var(--c-border)"
              strokeWidth={1}
            />
            <text x={PAD.left - 6} y={y(tick)} dy="0.32em" textAnchor="end" className="index-chart-axis">
              {tick.toFixed(0)}
            </text>
          </g>
        ))}
        {labelIndexes(points.length, X_LABELS).map((i, k, all) => (
          <text
            key={i}
            x={x(i)}
            y={CHART_H - 6}
            textAnchor={k === 0 ? "start" : k === all.length - 1 ? "end" : "middle"}
            className="index-chart-axis"
          >
            {points[i].date.slice(5)}
          </text>
        ))}
        {runs(points).map(({ dashed, from, to }) => (
          <polyline
            key={from}
            className="index-chart-line"
            points={points
              .slice(from, to + 1)
              .map((point, k) => `${x(from + k).toFixed(1)},${y(point.price).toFixed(1)}`)
              .join(" ")}
            fill="none"
            stroke="var(--c-primary)"
            strokeWidth={2}
            strokeDasharray={dashed ? "4 3" : undefined}
            strokeLinejoin="round"
            strokeLinecap="round"
          />
        ))}
        {markers.map((i) => (
          <line
            key={`marker-${i}`}
            className="price-chart-marker"
            x1={x(i)}
            x2={x(i)}
            y1={PAD.top + innerH - 6}
            y2={PAD.top + innerH}
            stroke="var(--c-text-3)"
            strokeWidth={1.5}
          />
        ))}
        {hovered && active != null && (
          <g>
            <line
              x1={x(active)}
              x2={x(active)}
              y1={PAD.top}
              y2={PAD.top + innerH}
              stroke="var(--c-text-3)"
              strokeWidth={1}
            />
            <circle
              cx={x(active)}
              cy={y(hovered.price)}
              r={4}
              fill="var(--c-primary)"
              stroke="var(--c-card)"
              strokeWidth={2}
            />
          </g>
        )}
      </svg>
      {hovered && active != null && (
        <div
          role="status"
          className="index-chart-tooltip"
          style={
            x(active) > width / 2
              ? { right: width - x(active) + 8 }
              : { left: x(active) + 8 }
          }
        >
          <strong>{hovered.price.toFixed(0)}</strong>
          <span>{hovered.date}</span>
          {hovered.notes?.map((note) => <span key={note}>{note}</span>)}
        </div>
      )}
    </div>
  );
}

