/**
 * 煤池卡片点开的单煤估价走势: 共享煤库口径的日度估价 (实时快照定格 + 事后补算),
 * 补算段画虚线, 对标基准换了的那天在横轴上方画竖标. 外壳与图与行情条的指数弹层共用.
 */
import { useEffect, useState } from "react";
import { ChartDialog, DEFAULT_RANGE, RangeButtons, rangeDays } from "./ChartDialog";
import { fetchCoalPriceHistory } from "./coalPrices";
import { lastDays } from "./mcci";
import { PriceChart, type ChartPoint } from "./PriceChart";
import type { CoalPriceHistory, CoalPricePoint } from "./types";

type Row = ChartPoint & { benchmark: string | null };

function toRow(point: CoalPricePoint): Row {
  const backfill = point.source === "backfill";
  return {
    date: point.day,
    price: point.price,
    benchmark: point.benchmark,
    dashed: backfill,
    notes: [point.benchmark ? `对标 ${point.benchmark}` : "按市场价估", ...(backfill ? ["回算"] : [])],
  };
}

const yuan = (price: number) => Math.round(price).toLocaleString("zh-CN");

export function CoalPriceDialog({ name, modified, onClose }: {
  name: string;
  /** 用户改过这种煤的化验值: 卡片价按修改值现算, 历史仍是共享煤库口径. */
  modified: boolean;
  onClose: () => void;
}) {
  const [loaded, setLoaded] = useState<{ name: string; history: CoalPriceHistory | null } | null>(null);
  const history = loaded?.name === name ? loaded.history : "loading";
  const [range, setRange] = useState(DEFAULT_RANGE);
  useEffect(() => {
    let alive = true;
    fetchCoalPriceHistory(name).then((result) => {
      if (alive) setLoaded({ name, history: result });
    });
    return () => {
      alive = false;
    };
  }, [name]);

  const all = history && history !== "loading" ? history.points.map(toRow) : [];
  const rows = lastDays(all, rangeDays(range));
  const latest = all[all.length - 1];
  const markers = rows.flatMap((row, i) => (i > 0 && row.benchmark !== rows[i - 1].benchmark ? [i] : []));
  const prices = rows.map((row) => row.price);

  const body =
    history === "loading" ? <p className="index-chart-empty">加载中…</p>
    : history === null ? <p className="index-chart-empty">走势暂时取不到</p>
    : (
      <>
        {rows.length >= 2 && <div className="index-chart-summary">
          <span className="index-chart-price">{yuan(latest.price)}</span>
          <span>元/吨 · {latest.date}</span>
          <span>最高 {yuan(Math.max(...prices))} · 最低 {yuan(Math.min(...prices))}</span>
        </div>}
        <RangeButtons range={range} onChange={setRange} />
        {rows.length < 2 ? (
          <p className="index-chart-empty">数据不足两天, 画不出走势</p>
        ) : (
          <PriceChart key={range} label={`${name}估价`} points={rows} markers={markers} />
        )}
        {rows.length >= 2 && rows.some((row) => row.dashed) && (
          <p className="price-chart-note">虚线为回算: 按当天能拿到的指数和报价补算</p>
        )}
        {modified && <p className="price-chart-note">历史为共享煤库口径, 不含你的修改</p>}
      </>
    );

  return (
    <ChartDialog
      titleId="coal-price-title"
      title={name}
      subtitle={latest ? (latest.benchmark ? `对标 ${latest.benchmark}` : "按市场价估") : "估价走势"}
      onClose={onClose}
    >
      {body}
    </ChartDialog>
  );
}
