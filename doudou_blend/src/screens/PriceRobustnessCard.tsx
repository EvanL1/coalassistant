import type { PriceSwing, RobustnessReport } from "../types";

const cell = { textAlign: "right" as const, padding: "4px 0 4px 8px" };
const note = { fontSize: 11, color: "var(--c-text-3)", marginTop: 8 };
const badge = {
  marginLeft: 4,
  padding: "0 4px",
  borderRadius: 4,
  fontSize: 10,
  background: "#fef3c7",
  color: "#92400e",
};
const typeTag = { marginLeft: 4, fontSize: 11, color: "var(--c-text-3)" };

/** 门槛是 5 元网格上的格点; 落在最低一格表示"不到 5 元". */
function yuan(threshold: number | null): string {
  if (threshold == null) return "—";
  return threshold <= 5 ? "≤5" : threshold.toFixed(0);
}

function shareText(share: number): string {
  if (share <= 0) return "未入选";
  if (share < 0.005) return "<0.5%";
  return `${(share * 100).toFixed(1)}%`;
}

function SwingRow({ swing }: { swing: PriceSwing }) {
  return (
    <tr>
      <td style={{ padding: "4px 0" }}>
        <span>{swing.name}</span>
        {swing.coal_type && <span style={typeTag}>{swing.coal_type}</span>}
        {swing.ask && <span style={badge}>先问价</span>}
      </td>
      <td style={cell}>{shareText(swing.share)}</td>
      <td style={cell}>{yuan(swing.rise)}</td>
      <td style={cell}>{yuan(swing.fall)}</td>
      <td style={cell}>±{swing.uncertainty.toFixed(0)}</td>
    </tr>
  );
}

function RobustSection({
  report,
  types,
}: {
  report: RobustnessReport;
  types: Map<string, string>;
}) {
  const robust = report.robust;
  if (!robust) {
    return (
      <p style={note}>
        没找到明显更稳的配方: 多花 {report.tolerance.toFixed(0)}{" "}
        元/吨以内, 换不出更多用价格有把握的煤的配方.
      </p>
    );
  }
  const mix = Object.entries(robust.recipe)
    .filter(([, ratio]) => ratio > 1e-4)
    .sort((a, b) => b[1] - a[1]);
  return (
    <div style={{ marginTop: 12 }}>
      <div style={{ fontSize: 13, fontWeight: 600 }}>
        更稳的配方: 按当前估算价多花 {robust.extra_cost.toFixed(1)} 元/吨,
        平均估价误差从 ±{report.price_error.toFixed(0)} 降到 ±
        {robust.price_error.toFixed(0)}
      </div>
      <div style={{ fontSize: 13, marginTop: 4 }}>
        {mix.map(([name, ratio], index) => (
          <span key={name}>
            {index > 0 && "、"}
            {name}
            {types.get(name) && <span style={typeTag}>{types.get(name)}</span>}{" "}
            {(ratio * 100).toFixed(1)}%
          </span>
        ))}
      </div>
      <p style={note}>
        做法: 给估价越不准的煤加越多的风险溢价再求解. 只按估算价求解时, LP
        会专挑估得偏低的煤; 加溢价正好抵消这个偏差, 结果更多用价格有把握的煤.
      </p>
    </div>
  );
}

/**
 * 估价可靠度卡: 煤价都是估出来的, 这个配方还靠不靠得住.
 * 翻转门槛 = 单种煤价格变动多少元, 配方要换掉一成以上吨位. "先问价"由核心标: 门槛小于
 * 估价误差 (估错就改配方), 且按"占比 × 误差"排在最前的几种. 没有 price_robustness 时
 * 整卡不渲染.
 */
export function PriceRobustnessCard({
  report,
  types,
}: {
  report: RobustnessReport | null | undefined;
  types: Map<string, string>;
}) {
  if (!report) return null;
  const ask = report.swings.filter((swing) => swing.ask);
  return (
    <div className="card price-robustness-card" style={{ marginTop: 12 }}>
      <div className="card-title">估价可靠度</div>
      <p style={{ fontSize: 13, margin: "4px 0 8px" }}>
        {ask.length > 0
          ? `先问 ${ask.map((swing) => swing.name).join("、")} 的价: 它们的价格只要差一点配方就会变, 差额比估价误差还小, 而且在配方里分量最重.`
          : "各煤的估价误差都不足以改变配方, 可以按这个配方去问价."}
      </p>
      {report.swings.length > 0 && (
        <table
          style={{ width: "100%", fontSize: 13, borderCollapse: "collapse" }}
        >
          <thead>
            <tr style={{ fontSize: 11, color: "var(--c-text-3)" }}>
              <th style={{ textAlign: "left", fontWeight: "normal" }}>煤名</th>
              <th style={{ ...cell, fontWeight: "normal" }}>占比</th>
              <th style={{ ...cell, fontWeight: "normal" }}>贵多少会变</th>
              <th style={{ ...cell, fontWeight: "normal" }}>便宜多少会变</th>
              <th style={{ ...cell, fontWeight: "normal" }}>估价误差</th>
            </tr>
          </thead>
          <tbody>
            {report.swings.map((swing) => (
              <SwingRow key={swing.name} swing={swing} />
            ))}
          </tbody>
        </table>
      )}
      <p style={note}>
        "会变" = 配方要换掉一成以上的吨位, 按 5 元一档试, "≤5" 即不到 5 元; 搜索范围 ±
        {report.swing_range.toFixed(0)} 元 (便宜不低于 0 元), 超出记 "—". 未入选的煤只看便宜多少会入选.
        估价误差来自公开报价回测, 按煤种取. 配方用煤的平均估价误差 ±
        {report.price_error.toFixed(0)} 元/吨 —— 不是成本误差: 各煤一起偏高或偏低只改总价, 不改配方. 单位 元/吨.
      </p>
      <RobustSection report={report} types={types} />
    </div>
  );
}
