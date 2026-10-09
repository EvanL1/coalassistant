import { INDICATOR_LABEL } from "../types";
import type { BlendResult, CoalValue } from "../types";

const cell = { textAlign: "right" as const, padding: "4px 0 4px 8px" };
const tag = {
  marginLeft: 4,
  padding: "0 4px",
  borderRadius: 4,
  fontSize: 10,
  background: "#fef3c7",
  color: "#92400e",
};

const label = (key: string) => INDICATOR_LABEL[key] ?? key;

/** 差额文案: 正 = 偏贵, 负 = 划算; 取整后为 0 记持平. */
function gapText(gap: number): string {
  const rounded = Math.round(Math.abs(gap));
  if (rounded === 0) return "持平";
  return `${gap > 0 ? "偏贵" : "划算"} ${rounded}`;
}

/** 颜色与文案一致: 取整后持平用中性色, 否则偏贵红、划算绿. */
function gapColor(gap: number): string {
  if (Math.round(Math.abs(gap)) === 0) return "var(--c-text-3)";
  return gap > 0 ? "var(--c-danger, #b91c1c)" : "var(--c-success, #15803d)";
}

function CoalValueRow({
  value,
  showBlendValue,
}: {
  value: CoalValue;
  showBlendValue: boolean;
}) {
  const missing = value.missing.map(label);
  return (
    <tr>
      <td style={{ padding: "4px 0" }}>
        <span>{value.name}</span>
        {value.extrapolated.length > 0 && (
          <span
            style={tag}
            title={`超出系数线性区间: ${value.extrapolated.map(label).join("、")}`}
          >
            外推
          </span>
        )}
        {missing.length > 0 && (
          <span style={tag}>{`缺 ${missing.join("、")}`}</span>
        )}
      </td>
      <td style={cell}>{value.fob != null ? value.fob.toFixed(0) : "—"}</td>
      <td style={cell}>{value.points.toFixed(0)}</td>
      <td style={cell}>{value.fair.toFixed(0)}</td>
      {value.gap != null ? (
        <td style={{ ...cell, color: gapColor(value.gap) }}>
          {gapText(value.gap)}
        </td>
      ) : (
        <td style={{ ...cell, color: "var(--c-text-3)" }}>—</td>
      )}
      {showBlendValue && (
        <td style={cell}>
          {value.blend_value != null ? value.blend_value.toFixed(0) : "—"}
        </td>
      )}
    </tr>
  );
}

/**
 * 煤价对比卡: 每种煤的出厂价 vs 按质量算出的应值价.
 * 质量指数以标准品 = 1000 点计; 应值 = 市场价/1000 × 质量指数.
 * 出厂价取送进求解的口径 (发生锚点推算时是推算价, 不是录入的原始报价), drifted 时脚注说明.
 * 只是评估, 与配方无关; 有解无解都可显示. 没有 coal_values 时整卡不渲染.
 */
export function CoalValueCard({
  result,
  drifted = false,
}: {
  result: BlendResult;
  drifted?: boolean;
}) {
  // 按质量指数从高到低排, 好煤在上.
  const values = [...(result.coal_values ?? [])].sort(
    (a, b) => b.points - a.points,
  );
  const pricedCount = values.filter((value) => value.fob != null).length;
  const showBlendValue = values.some((value) => value.blend_value != null);
  if (values.length === 0) return null;
  const market = result.market_price;
  return (
    <div className="card coal-value-card" style={{ marginTop: 12 }}>
      <div className="card-title">煤价对比 (按质量)</div>
      <table
        style={{ width: "100%", fontSize: 13, borderCollapse: "collapse" }}
      >
        <thead>
          <tr style={{ fontSize: 11, color: "var(--c-text-3)" }}>
            <th style={{ textAlign: "left", fontWeight: "normal" }}>煤名</th>
            <th style={{ ...cell, fontWeight: "normal" }}>出厂价 (求解口径)</th>
            <th style={{ ...cell, fontWeight: "normal" }}>质量指数</th>
            <th style={{ ...cell, fontWeight: "normal" }}>按质量应值</th>
            <th style={{ ...cell, fontWeight: "normal" }}>差额</th>
            {showBlendValue && (
              <th style={{ ...cell, fontWeight: "normal" }}>配煤价值 (到厂)</th>
            )}
          </tr>
        </thead>
        <tbody>
          {values.map((value) => (
            <CoalValueRow
              key={value.name}
              value={value}
              showBlendValue={showBlendValue}
            />
          ))}
        </tbody>
      </table>
      <p style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 8 }}>
        {market != null &&
          `市场价 ${market.toFixed(0)} 元/吨 = 标准品 1000 点 (${
            result.market_price_from_group
              ? `由 ${pricedCount} 种有报价的煤反推`
              : "指定"
          }) · `}
        只用出厂价, 不含运费
        {drifted && " · 有煤用旧报价推算, 按推算后的出厂价对比"} ·
        未录出厂价的煤只给估算价 · 单位 元/吨
      </p>
      {showBlendValue && (
        <p style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 4 }}>
          配煤价值按本次合同与煤池算: 到厂价低于配煤价值就会被选进配方,
          已入选的煤即其现价; 只按少量掺入算, 买多了价值会下降. 估算价看市场,
          配煤价值看你这一炉.
        </p>
      )}
    </div>
  );
}
