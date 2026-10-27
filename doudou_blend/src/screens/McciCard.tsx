/**
 * 炼焦煤价格指数卡片 (界面不标数据来源): 每条序列一行 —— 品种与标准品指标、最新值与日期、涨跌、近一年走势.
 *
 * 估价按同煤种的指数对标, 这张卡让人看得到"对标的是什么规格、今天多少、最近怎么走".
 * 走势是每行各自缩放的小图 (只看形状), 旁边写出一年高低, 幅度不靠目测.
 * 接口取不到时整张卡不显示.
 */
import { useEffect, useState } from "react";
import { changeOver, fetchMcci, lastYear, specText, staleDays } from "../mcci";
import type { McciPoint, McciSeries } from "../types";

/** 超过这么多个日历天没新值就提示 (周末 + 一天假期以内不算). */
export const STALE_AFTER_DAYS = 4;
const CHART_W = 120;
const CHART_H = 28;
/** 两端留白, 末端圆点与悬停圆点不被裁掉. */
const CHART_PAD = 4;

const small = { fontSize: 11, color: "var(--c-text-3)" };
const cell = { padding: "6px 6px 6px 0", verticalAlign: "top" as const };
/** 数字列不折行 (手机宽度下日期、涨跌被拆成两行很难读). */
const numCell = { ...cell, textAlign: "right" as const, whiteSpace: "nowrap" as const };

const shortDate = (date: string) => date.slice(5);

/** 国内习惯: 红涨绿跌, 平盘中性. */
export function ChangeText({ pct }: { pct: number | null }) {
  if (pct == null) return <span style={small}>—</span>;
  const rounded = Number(pct.toFixed(2));
  const color =
    rounded === 0 ? "var(--c-text-3)" : rounded > 0 ? "var(--c-danger)" : "var(--c-success)";
  const arrow = rounded === 0 ? "" : rounded > 0 ? "▲+" : "▼";
  return (
    <span style={{ color, fontWeight: 600 }}>
      {arrow}
      {Math.abs(rounded).toFixed(2)}%
    </span>
  );
}

/** 一年走势小图: 悬停出十字线与当天的值. */
function Trend({ label, points }: { label: string; points: McciPoint[] }) {
  const [hover, setHover] = useState<number | null>(null);
  const prices = points.map((point) => point.price);
  const min = Math.min(...prices);
  const max = Math.max(...prices);
  const span = max - min || 1;
  const x = (i: number) =>
    CHART_PAD + (i / Math.max(points.length - 1, 1)) * (CHART_W - 2 * CHART_PAD);
  const y = (price: number) => CHART_H - 3 - ((price - min) / span) * (CHART_H - 6);
  const line = points.map((point, i) => `${x(i).toFixed(1)},${y(point.price).toFixed(1)}`).join(" ");
  const last = points.length - 1;
  const hovered = hover != null ? points[hover] : null;

  return (
    <div>
      <svg
        role="img"
        aria-label={`${label} 近一年走势, 低 ${min.toFixed(0)} 高 ${max.toFixed(0)}`}
        width={CHART_W}
        height={CHART_H}
        viewBox={`0 0 ${CHART_W} ${CHART_H}`}
        style={{ display: "block", cursor: "crosshair", touchAction: "pan-y" }}
        onPointerMove={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          const width = rect.width || CHART_W;
          const inner = (width * (CHART_W - 2 * CHART_PAD)) / CHART_W;
          const offset = (width * CHART_PAD) / CHART_W;
          const ratio = Math.min(Math.max((event.clientX - rect.left - offset) / inner, 0), 1);
          setHover(Math.round(ratio * last));
        }}
        onPointerLeave={() => setHover(null)}
      >
        <polyline points={line} fill="none" stroke="var(--c-text-3)" strokeWidth={1.5} />
        <circle cx={x(last)} cy={y(points[last].price)} r={2.5} fill="var(--c-primary)" />
        {hovered && hover != null && (
          <g>
            <line x1={x(hover)} x2={x(hover)} y1={0} y2={CHART_H} stroke="var(--c-border)" />
            <circle
              cx={x(hover)}
              cy={y(hovered.price)}
              r={3}
              fill="var(--c-primary)"
              stroke="var(--c-card)"
              strokeWidth={1.5}
            />
          </g>
        )}
      </svg>
      <div style={{ ...small, whiteSpace: "nowrap" }}>
        {hovered
          ? `${shortDate(hovered.date)} · ${hovered.price.toFixed(0)}`
          : `低 ${min.toFixed(0)} · 高 ${max.toFixed(0)}`}
      </div>
    </div>
  );
}

function SeriesRow({ series }: { series: McciSeries }) {
  const points = lastYear(series.points);
  const latest = points[points.length - 1];
  const stale = latest ? staleDays(latest.date) : 0;
  return (
    <tr>
      <td style={cell}>
        <div>{series.type_name}</div>
        <div style={small}>{specText(series.props)}</div>
      </td>
      <td style={numCell}>
        {latest ? (
          <>
            <div style={{ fontWeight: 700 }}>{latest.price.toFixed(0)}</div>
            <div style={small}>{shortDate(latest.date)}</div>
            {stale > STALE_AFTER_DAYS && (
              <div style={{ fontSize: 11, color: "var(--c-warning)" }}>已 {stale} 天未更新</div>
            )}
          </>
        ) : (
          <span style={small}>—</span>
        )}
      </td>
      <td style={numCell}>
        <ChangeText pct={changeOver(points, 0)} />
        <div style={small}>
          周 <ChangeText pct={changeOver(points, 7)} />
        </div>
      </td>
      <td style={cell}>
        {points.length >= 2 ? (
          <Trend label={series.type_name} points={points} />
        ) : (
          <span style={small}>{points.length === 0 ? "还没拉到数据" : "数据不足"}</span>
        )}
      </td>
    </tr>
  );
}

export function McciCard() {
  const [series, setSeries] = useState<McciSeries[] | null>(null);

  useEffect(() => {
    let alive = true;
    void fetchMcci().then((next) => {
      if (alive) setSeries(next);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (!series || series.length === 0) return null;

  return (
    <div className="card" style={{ marginTop: 12 }}>
      <div className="card-title">炼焦煤价格指数</div>
      <div style={{ ...small, marginBottom: 8 }}>
        每个品种是一个固定规格的标准品 (山西, 元/湿吨, 现金). 估价按同煤种的指数对标,
        每晚 17:30 后更新.
      </div>
      <div style={{ overflowX: "auto" }}>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
          <thead>
            <tr style={{ ...small, textAlign: "left" }}>
              <th style={cell}>品种 · 标准品</th>
              <th style={{ ...cell, textAlign: "right" }}>最新</th>
              <th style={{ ...cell, textAlign: "right" }}>日涨跌</th>
              <th style={cell}>近一年</th>
            </tr>
          </thead>
          <tbody>
            {series.map((row) => (
              <SeriesRow key={row.code} series={row} />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
