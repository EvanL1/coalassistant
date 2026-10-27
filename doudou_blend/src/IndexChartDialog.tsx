/**
 * 行情条点开的指数走势弹层: 一个品种的完整走势 (最多约一年), 可切时间范围,
 * 十字线 + tooltip 读任一天的值; 键盘 ←/→/Home/End 同样能逐日读.
 *
 * 单条序列, 不放图例 (标题就是名字); 网格与坐标轴用边框/弱文字色, 线用主色.
 * 图宽跟容器走 (手机全宽), 高度固定. 界面不标数据来源.
 */
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { changeOver, lastDays, specText } from "./mcci";
import { ChangeText } from "./screens/McciCard";
import type { McciPoint, McciSeries } from "./types";

/** 时间范围 (日历天). */
const RANGES: [label: string, days: number][] = [
  ["1月", 30],
  ["3月", 91],
  ["6月", 182],
  ["1年", 365],
];
const DEFAULT_RANGE = "1年";

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

function PriceChart({ label, points }: { label: string; points: McciPoint[] }) {
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
  const line = points.map((point, i) => `${x(i).toFixed(1)},${y(point.price).toFixed(1)}`).join(" ");
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
        <polyline
          className="index-chart-line"
          points={line}
          fill="none"
          stroke="var(--c-primary)"
          strokeWidth={2}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
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
        </div>
      )}
    </div>
  );
}

export function IndexChartDialog({
  series,
  onClose,
}: {
  series: McciSeries;
  onClose: () => void;
}) {
  const [range, setRange] = useState(DEFAULT_RANGE);
  const closeRef = useRef<HTMLButtonElement>(null);
  const days = RANGES.find(([label]) => label === range)?.[1] ?? 365;
  const points = lastDays(series.points, days);
  const latest = series.points[series.points.length - 1];

  // 父组件每次渲染都给新的 onClose; 走 ref, 焦点处理只在开、关时各做一次.
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  // 打开时焦点进弹层, 关闭后回到点开它的那一项; Esc 关闭.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") onCloseRef.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      opener?.focus?.();
    };
  }, []);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="index-chart-title"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-handle" />
        <div className="modal-header">
          <div>
            <div className="modal-title" id="index-chart-title">
              {series.type_name}
            </div>
            <div className="modal-subtitle">标准品 {specText(series.props)}</div>
          </div>
          <button ref={closeRef} className="modal-close" aria-label="关闭" onClick={onClose}>
            ×
          </button>
        </div>
        <div className="modal-body index-chart-body">
          {latest && (
            <div className="index-chart-summary">
              <span className="index-chart-price">{latest.price.toFixed(0)}</span>
              <span>元/吨 · {latest.date}</span>
              <span>
                日 <ChangeText pct={changeOver(series.points, 0)} />
              </span>
              <span>
                周 <ChangeText pct={changeOver(series.points, 7)} />
              </span>
            </div>
          )}
          <div className="index-chart-ranges" role="group" aria-label="时间范围">
            {RANGES.map(([label]) => (
              <button
                key={label}
                type="button"
                aria-pressed={label === range}
                className={label === range ? "active" : undefined}
                onClick={() => setRange(label)}
              >
                {label}
              </button>
            ))}
          </div>
          {points.length >= 2 ? (
            <PriceChart key={range} label={series.type_name} points={points} />
          ) : (
            <p className="index-chart-empty">这段时间不足两个交易日, 画不出走势.</p>
          )}
        </div>
      </div>
    </div>
  );
}
