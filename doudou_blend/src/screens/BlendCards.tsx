import { INDICATOR_LABEL, INDICATOR_ORDER } from "../types";
import type { BlendResult, EvaluationMethod, EvaluationStatus, IndicatorCheck, QualityStatus } from "../types";

const RECIPE_COLORS = ["#0a5fff", "#7c3aed", "#ec4899", "#f59e0b", "#10b981", "#06b6d4", "#ef4444", "#8b5cf6"];

export const QUALITY_STATUS_LABEL: Record<QualityStatus, string> = {
  Verified: "已验证",
  Estimated: "估算",
  NeedsReview: "需复核",
};

const EVALUATION_STATUS_LABEL: Record<EvaluationStatus, string> = {
  Pass: "通过",
  TolerancePass: "容差通过",
  Unverified: "未验证",
  Fail: "未通过",
};

const EVALUATION_METHOD_LABEL: Record<EvaluationMethod, string> = {
  Linear: "线性加权",
  ProvisionalLinear: "暂行线性",
  AffineCalibration: "仿射校准",
  Histogram: "直方图复验",
  Moments: "统计矩复验",
  Regression: "回归预测",
  Unavailable: "无法评估",
};

function legacyEvaluationStatus(check: IndicatorCheck): EvaluationStatus {
  return check.slack != null && check.slack < -0.01 ? "Fail" : "Pass";
}

function evaluationStatus(check: IndicatorCheck): EvaluationStatus {
  return check.status ?? legacyEvaluationStatus(check);
}

export function isIndicatorPassing(check: IndicatorCheck): boolean {
  const status = evaluationStatus(check);
  return status !== "Fail" && check.method !== "Unavailable";
}

export function isContractIndicator(check: IndicatorCheck): boolean {
  return check.min != null || check.max != null;
}

function formatIndicatorValue(value: number): string {
  return value
    .toFixed(4)
    .replace(/\.?0+$/, "");
}

export function RecipeCard({
  result,
  coalTypes,
  title = "今日配方",
}: {
  result: BlendResult;
  coalTypes: Map<string, string>;
  title?: string;
}) {
  const sortedRecipe = [...result.orders].sort((a, b) => b.ratio - a.ratio);
  return (
    <div className="card today-recipe">
      <div
        className="card-title"
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <span>{title}</span>
        <span style={{ fontSize: 11, color: "var(--c-text-3)" }}>
          选用 {sortedRecipe.length} 种煤
        </span>
      </div>
      <div className="recipe-bar">
        {sortedRecipe.map((o, i) => (
          <div
            key={o.coal}
            className="recipe-seg"
            style={{
              background: RECIPE_COLORS[i % RECIPE_COLORS.length],
              width: `${o.ratio * 100}%`,
            }}
            title={`${o.coal} ${(o.ratio * 100).toFixed(1)}%`}
          >
            {o.ratio > 0.1 ? `${(o.ratio * 100).toFixed(0)}%` : ""}
          </div>
        ))}
      </div>
      <div className="recipe-list">
        {sortedRecipe.map((o, i) => (
          <div key={o.coal} className="recipe-row">
            <div className="recipe-row-left">
              <span
                className="recipe-dot"
                style={{ background: RECIPE_COLORS[i % RECIPE_COLORS.length] }}
              />
              <span>{o.coal}</span>
              {coalTypes.get(o.coal) && (
                <span className="recipe-coal-type">{coalTypes.get(o.coal)}</span>
              )}
            </div>
            <div className="recipe-amount">
              {(o.ratio * 100).toFixed(2)}% ·{" "}
              {o.tons != null ? `${o.tons.toFixed(0)} 吨` : "-"}
              {o.shovels != null && ` · ${o.shovels} 铲`}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export function IndicatorCard({ result }: { result: BlendResult }) {
  const qualityStatus = result.quality_status ?? "Estimated";
  return (
    <div className="card today-indicators">
      <div
        className="card-title"
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <span>混合指标</span>
        <span style={{ fontSize: 11, color: "var(--c-text-3)" }}>
          {QUALITY_STATUS_LABEL[qualityStatus]}
          {result.evaluation_iterations != null
            ? ` · ${result.evaluation_iterations} 轮评估`
            : ""}
        </span>
      </div>
      <div className="indicator-grid">
        {INDICATOR_ORDER.map((key) => {
          const ic = result.indicator_check.find((c) => c.indicator === key);
          const label = INDICATOR_LABEL[key];
          if (!ic) {
            return (
              <div key={key} className="indicator-cell unconstrained">
                <div className="indicator-label">{label}</div>
                <div className="indicator-value" style={{ color: "var(--c-text-3)" }}>
                  —
                </div>
                <div className="indicator-meta">未约束</div>
              </div>
            );
          }
          const status = evaluationStatus(ic);
          const violated = status === "Fail";
          const unverified = status === "Unverified";
          const className = `indicator-cell ${
            violated
              ? "violated"
              : unverified
                ? "unverified"
                : ic.binding
                  ? "binding"
                  : ""
          }`;
          const rawValue = ic.evaluated_value ?? ic.value;
          const judgedValue = ic.judged_value ?? rawValue;
          const proxyValue = ic.proxy_value;
          const method = ic.method ?? "Linear";
          const unavailable =
            method === "Unavailable" && ic.evaluated_value == null;
          let rangeStr = "—";
          if (ic.min != null && ic.max != null)
            rangeStr = `${ic.min}-${ic.max}`;
          else if (ic.min != null) rangeStr = `≥${ic.min}`;
          else if (ic.max != null) rangeStr = `≤${ic.max}`;
          return (
            <div key={key} className={className}>
              <div className="indicator-label">{label}</div>
              <div className="indicator-value-row">
                <div className="indicator-value">
                  {unavailable ? "—" : formatIndicatorValue(judgedValue)}
                </div>
                {ic.penalty_per_ton != null && ic.penalty_per_ton > 1e-6 ? (
                  <span className="indicator-penalty">
                    扣 {ic.penalty_per_ton.toFixed(2)} 元/吨
                  </span>
                ) : (
                  <span
                    className={`evaluation-pill evaluation-${status.toLowerCase()}`}
                  >
                    {EVALUATION_STATUS_LABEL[status]}
                  </span>
                )}
              </div>
              <div className="indicator-meta">
                {unavailable
                  ? "缺少可用指标或评估输入"
                  : `原始 ${formatIndicatorValue(rawValue)} · 判定 ${formatIndicatorValue(judgedValue)}`}
              </div>
              <div className="indicator-meta">
                {rangeStr}
                {ic.binding && !violated ? " · ★ 顶格" : ""}
                {ic.uncertainty != null
                  ? ` · ±${formatIndicatorValue(ic.uncertainty)}`
                  : ""}
              </div>
              <div className="indicator-meta indicator-method">
                {EVALUATION_METHOD_LABEL[method]}
                {proxyValue != null &&
                Math.abs(proxyValue - rawValue) > 0.000001
                  ? ` · 代理 ${formatIndicatorValue(proxyValue)}`
                  : ""}
                {ic.model?.version ? ` · ${ic.model.version}` : ""}
                {ic.model?.in_domain === false ? " · 训练域外" : ""}
              </div>
              {/* 第二个 CSR 估计: 按配合煤挥发/G/Y 推 (捣固), 与上面的单煤加权并列.
                  公式精度在个位数, 只给一位小数. */}
              {key === "CSR" && result.csr_stamp_estimate != null && (
                <div className="indicator-meta">
                  捣固公式估算 {result.csr_stamp_estimate.toFixed(1)}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
