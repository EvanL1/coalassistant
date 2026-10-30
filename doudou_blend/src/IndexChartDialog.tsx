/** 行情条点开的指数走势, 默认显示一年. */
import { useState } from "react";
import { ChartDialog, DEFAULT_RANGE, RangeButtons, rangeDays } from "./ChartDialog";
import { changeOver, lastDays, specText } from "./mcci";
import { PriceChart } from "./PriceChart";
import { ChangeText } from "./screens/McciCard";
import type { McciSeries } from "./types";

export function IndexChartDialog({ series, onClose }: { series: McciSeries; onClose: () => void }) {
  const [range, setRange] = useState(DEFAULT_RANGE);
  const points = lastDays(series.points, rangeDays(range));
  const latest = series.points[series.points.length - 1];

  return (
    <ChartDialog titleId="index-chart-title" title={series.type_name}
      subtitle={`标准品 ${specText(series.props)}`} onClose={onClose}>
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
      <RangeButtons range={range} onChange={setRange} />
      {points.length >= 2 ? (
        <PriceChart key={range} label={series.type_name} points={points} />
      ) : (
        <p className="index-chart-empty">这段时间不足两个交易日, 画不出走势.</p>
      )}
    </ChartDialog>
  );
}
