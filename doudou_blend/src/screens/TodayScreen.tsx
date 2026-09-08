/**
 * 屏 1 - 今日（首页）
 *
 * 数据源:
 *   - 煤池: master + user_coal_prefs (启用 + 价格 + 化验值 override)
 *   - 合同: user_contract ?? master.default_contract
 *   - 默认: master verified 的 4 主力煤启用, 其他停用
 *
 * 求解后展示成本、配方、8 项指标, 并支持保存到历史.
 */
import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { getBackend } from "../backend";
import {
  resolveCoalPool,
  summarizeDrift,
  toBlendCoal,
} from "../domain/resolvedCoal";
import { buildPriceAnchor } from "../domain/priceDrift";
import {
  isSnapshotActionable,
  LatestRequestTracker,
  type SolveSnapshot,
} from "../domain/solveSession";
import { loadMaster } from "../master_loader";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "../types";
import type {
  BlendRequest,
  BlendResult,
  EvaluationMethod,
  EvaluationStatus,
  IndicatorCheck,
  QualityStatus,
  Spec,
} from "../types";
import type { TabId } from "../TabBar";
import {
  getCoalPrefs,
  getQuantity,
  getUserContract,
  getUserCoals,
  setQuantity,
} from "../storage";

const RECIPE_COLORS = ["#0a5fff", "#7c3aed", "#ec4899", "#f59e0b", "#10b981", "#06b6d4", "#ef4444", "#8b5cf6"];

/** 输入摘要里的可点击链接 (跳合同/煤池). */
const summaryLink: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  cursor: "pointer",
  color: "var(--c-primary)",
  fontWeight: 600,
  fontSize: 12,
};

function formatPrice(n: number): { int: string; dec: string } {
  const [intPart, decPart] = n.toFixed(2).split(".");
  return { int: intPart, dec: decPart };
}

const QUALITY_STATUS_LABEL: Record<QualityStatus, string> = {
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

function isIndicatorPassing(check: IndicatorCheck): boolean {
  const status = evaluationStatus(check);
  return status !== "Fail" && check.method !== "Unavailable";
}

function isContractIndicator(check: IndicatorCheck): boolean {
  return check.min != null || check.max != null;
}

function formatIndicatorValue(value: number): string {
  return value
    .toFixed(4)
    .replace(/\.?0+$/, "");
}

/** 把求解结果格式化成可复制的采购清单文本 (纯函数). */
function buildOrderText(
  result: BlendResult,
  contractName: string,
  isoDate: string,
  quantity: number,
): string {
  const cost = result.cost;
  const contractChecks = result.indicator_check.filter(isContractIndicator);
  const total = contractChecks.length;
  const passing = contractChecks.filter(isIndicatorPassing).length;
  const yuan = (n: number) => `¥${Math.round(n).toLocaleString("zh-CN")}`;

  const lines: string[] = [];
  lines.push(`【豆哥配煤】${isoDate}`);
  lines.push(`合同: ${contractName || "默认合同"} | 总量 ${quantity} 吨`);
  if (cost) {
    const quality = QUALITY_STATUS_LABEL[result.quality_status ?? "Estimated"];
    lines.push(
      `到厂价 ${cost.cif_per_ton.toFixed(2)} 元/吨 | 数值界内 ${passing}/${total} | 质量 ${quality}`,
    );
  }
  lines.push("──────────────────");
  for (const o of [...result.orders].sort((a, b) => b.ratio - a.ratio)) {
    const pct = `${(o.ratio * 100).toFixed(1)}%`;
    const tons = o.tons != null ? `${Math.round(o.tons)}吨` : "-";
    const amt = o.cif_amount != null ? `  ${yuan(o.cif_amount)}` : "";
    lines.push(`${o.coal}  ${pct}  ${tons}${amt}`);
  }
  if (cost?.total_cif != null) {
    lines.push(`总额 ${yuan(cost.total_cif)}`);
  }
  return lines.join("\n");
}

/** 复制到剪贴板. 优先 Clipboard API, 不可用时回退临时 textarea (兼容老 webview). */
async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // 落到下面的兜底
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}

type SolveState =
  | { status: "loading" }
  | { status: "refreshing"; snapshot: SolveSnapshot }
  | { status: "ok"; snapshot: SolveSnapshot }
  | { status: "error"; error: string };

export function TodayScreen({ onNavigate }: { onNavigate: (tab: TabId) => void }) {
  const [state, setState] = useState<SolveState>({ status: "loading" });
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // 重算反馈: idle / running / done. running 时按钮显示"重算中...", done 时显示"✓ 已重算" 1.5s
  const [recompute, setRecompute] = useState<"idle" | "running" | "done">("idle");
  // 采购吨数输入 (字符串以便编辑); 导出反馈文案
  const [qtyInput, setQtyInput] = useState(() => String(getQuantity()));
  const [exportMsg, setExportMsg] = useState<string | null>(null);
  const qtyInputRef = useRef(qtyInput);
  const requestTrackerRef = useRef(new LatestRequestTracker());
  const recomputeTimerRef = useRef<number | null>(null);
  const saveTimerRef = useRef<number | null>(null);
  const exportTimerRef = useRef<number | null>(null);
  const savingRef = useRef(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    void runSolve(true);
    // 监听 prefs/contract/user_coals 变化, 自动重算 (inline, 不整页 loading)
    const refresh = () => void runSolve(false);
    window.addEventListener("doudou:prefs_changed", refresh);
    window.addEventListener("doudou:contract_changed", refresh);
    window.addEventListener("doudou:user_coals_changed", refresh);
    return () => {
      mountedRef.current = false;
      window.removeEventListener("doudou:prefs_changed", refresh);
      window.removeEventListener("doudou:contract_changed", refresh);
      window.removeEventListener("doudou:user_coals_changed", refresh);
      requestTrackerRef.current.invalidate();
      if (recomputeTimerRef.current != null) {
        window.clearTimeout(recomputeTimerRef.current);
      }
      if (saveTimerRef.current != null) {
        window.clearTimeout(saveTimerRef.current);
      }
      if (exportTimerRef.current != null) {
        window.clearTimeout(exportTimerRef.current);
      }
      savingRef.current = false;
    };
  }, []);

  /** initial=true 时整页 loading; 否则原地反馈, 保留旧结果直到新结果出来. */
  async function runSolve(initial: boolean = false) {
    const tracker = requestTrackerRef.current;
    const requestId = tracker.issue();
    // 用户输入必须在任何 await 前冻结，确保请求号对应唯一输入快照。
    const prefs = getCoalPrefs();
    const userContract = getUserContract();
    const userCoals = getUserCoals();
    const totalQuantity = getQuantity();

    if (recomputeTimerRef.current != null) {
      window.clearTimeout(recomputeTimerRef.current);
      recomputeTimerRef.current = null;
    }
    setSaveMsg(null);
    setExportMsg(null);
    if (initial) {
      setState({ status: "loading" });
      setRecompute("idle");
    } else {
      setRecompute("running");
      setState((previous) =>
        "snapshot" in previous
          ? { status: "refreshing", snapshot: previous.snapshot }
          : { status: "loading" },
      );
    }

    try {
      const [master, backend] = await Promise.all([loadMaster(), getBackend()]);
      if (!tracker.isCurrent(requestId)) return;

      // 用锚点煤把各煤的旧报价推算到当前口径再求解, 否则会系统性低估到厂成本.
      const anchor = buildPriceAnchor(prefs);
      const pool = resolveCoalPool(master.coals, userCoals, prefs, {
        anchor,
        masterUpdatedAt: master.updated_at,
      });
      const driftSummary = summarizeDrift(pool, anchor.coals);
      const coals = pool
        .map(toBlendCoal)
        .filter((coal) => coal != null);

      if (coals.length === 0) {
        tracker.accept(requestId, () => {
          setState({
            status: "error",
            error: "没有可参与求解的煤。请在煤池启用煤种并补齐有效价格。",
          });
        });
        return;
      }

      const specs: Spec[] = userContract ?? master.default_contract.specs;
      const contractName = userContract
        ? "用户自定义合同"
        : master.default_contract.name;

      const request: BlendRequest = {
        coals,
        specs,
        total_quantity: totalQuantity,
        truncate_decimal: true,
      };

      const json = await backend.solveJson(JSON.stringify(request));
      const result: BlendResult = JSON.parse(json);
      tracker.accept(requestId, () => {
        setState({
          status: "ok",
          snapshot: {
            requestId,
            request,
            result,
            contractName,
            enabledCount: coals.length,
            drift: driftSummary,
          },
        });
      });
    } catch (e) {
      tracker.accept(requestId, () => {
        setState({ status: "error", error: String(e) });
      });
    } finally {
      if (!initial && tracker.isCurrent(requestId)) {
        setRecompute("done");
        recomputeTimerRef.current = window.setTimeout(() => {
          if (tracker.isCurrent(requestId)) {
            setRecompute("idle");
          }
          recomputeTimerRef.current = null;
        }, 1500);
      }
    }
  }

  async function saveToHistory() {
    const snapshot = state.status === "ok" ? state.snapshot : null;
    if (
      savingRef.current ||
      !snapshot?.result.cost ||
      !isSnapshotActionable(
        snapshot,
        requestTrackerRef.current.currentRequestId,
        qtyInputRef.current,
      )
    ) {
      return;
    }

    savingRef.current = true;
    setSaving(true);
    setSaveMsg(null);
    if (saveTimerRef.current != null) {
      window.clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    let feedbackShown = false;
    try {
      const backend = await getBackend();
      if (
        !isSnapshotActionable(
          snapshot,
          requestTrackerRef.current.currentRequestId,
          qtyInputRef.current,
        )
      ) {
        return;
      }
      const quantity = snapshot.request.total_quantity;
      if (typeof quantity !== "number") return;
      // 保存完整快照，避免新吨数和旧订单组合成一条历史记录。
      await backend.saveHistory(
        snapshot.result,
        snapshot.contractName,
        quantity,
      );
      if (
        !mountedRef.current ||
        !requestTrackerRef.current.isCurrent(snapshot.requestId)
      ) {
        return;
      }
      setSaveMsg("✓ 已保存");
      feedbackShown = true;
    } catch (error) {
      console.error("保存方案失败", error);
      if (
        mountedRef.current &&
        requestTrackerRef.current.isCurrent(snapshot.requestId)
      ) {
        setSaveMsg("保存失败");
        feedbackShown = true;
      }
    } finally {
      savingRef.current = false;
      if (!mountedRef.current) return;
      setSaving(false);
      if (
        !feedbackShown ||
        !requestTrackerRef.current.isCurrent(snapshot.requestId)
      ) {
        return;
      }
      saveTimerRef.current = window.setTimeout(() => {
        if (
          mountedRef.current &&
          requestTrackerRef.current.isCurrent(snapshot.requestId)
        ) {
          setSaveMsg(null);
        }
        saveTimerRef.current = null;
      }, 2000);
    }
  }

  /** 提交采购吨数: 合法则持久化并重算, 非法则回退上次值. */
  function commitQty() {
    const n = Number(qtyInputRef.current);
    if (Number.isFinite(n) && n > 0) {
      const normalized = String(n);
      const currentSnapshot =
        state.status === "ok" ? state.snapshot : null;
      qtyInputRef.current = normalized;
      setQtyInput(normalized);
      if (
        isSnapshotActionable(
          currentSnapshot,
          requestTrackerRef.current.currentRequestId,
          normalized,
        )
      ) {
        return;
      }
      setQuantity(n);
      void runSolve(false);
    } else {
      const previous = String(getQuantity());
      qtyInputRef.current = previous;
      setQtyInput(previous);
    }
  }

  /** 导出: 把当前方案复制成采购清单文本. */
  async function exportOrder() {
    const snapshot = state.status === "ok" ? state.snapshot : null;
    if (
      snapshot == null ||
      !isSnapshotActionable(
        snapshot,
        requestTrackerRef.current.currentRequestId,
        qtyInputRef.current,
      )
    ) {
      return;
    }
    const quantity = snapshot.request.total_quantity;
    if (typeof quantity !== "number") return;
    const d = new Date();
    const iso = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    const text = buildOrderText(
      snapshot.result,
      snapshot.contractName,
      iso,
      quantity,
    );
    const ok = await copyText(text);
    if (
      !mountedRef.current ||
      !requestTrackerRef.current.isCurrent(snapshot.requestId)
    ) {
      return;
    }
    setExportMsg(ok ? "✓ 已复制清单" : "复制失败");
    if (exportTimerRef.current != null) {
      window.clearTimeout(exportTimerRef.current);
    }
    exportTimerRef.current = window.setTimeout(() => {
      if (
        mountedRef.current &&
        requestTrackerRef.current.isCurrent(snapshot.requestId)
      ) {
        setExportMsg(null);
      }
      exportTimerRef.current = null;
    }, 2000);
  }

  if (state.status === "loading") {
    return <div className="loading">求解中...</div>;
  }
  if (state.status === "error") {
    return (
      <div className="empty">
        <p style={{ marginBottom: 16 }}>{state.error}</p>
        <button className="btn btn-primary" onClick={() => runSolve(true)}>
          重试
        </button>
      </div>
    );
  }

  const { snapshot } = state;
  const { result, contractName, enabledCount } = snapshot;
  const drift = snapshot.drift ?? null;
  const refreshing = state.status === "refreshing";
  const actionsEnabled = isSnapshotActionable(
    snapshot,
    requestTrackerRef.current.currentRequestId,
    qtyInput,
  );
  if (!result.ok) {
    return (
      <>
        <div className="page-header">
          <div>
            <h1 className="page-title">今日配煤</h1>
            <div className="page-subtitle">{contractName}</div>
          </div>
        </div>
        <div
          className="card"
          style={{ borderLeft: "4px solid var(--c-danger)" }}
        >
          <div className="card-title" style={{ color: "var(--c-danger)" }}>
            ✗ 不可行
          </div>
          <p style={{ margin: 0, fontSize: 13 }}>{result.reason}</p>
          {result.warnings.length > 0 && (
            <ul style={{ fontSize: 12, color: "var(--c-text-3)", paddingLeft: 18 }}>
              {result.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          )}
        </div>
        <p style={{ fontSize: 12, color: "var(--c-text-3)", marginTop: 12 }}>
          建议: 去「合同」放宽某项约束, 或去「煤池」启用更多煤源.
        </p>
        <div className="action-row">
          <button
            className="btn btn-secondary"
            onClick={() => runSolve(false)}
            disabled={recompute === "running"}
          >
            {recompute === "running" ? "重算中..." : recompute === "done" ? "✓ 已重算" : "重试"}
          </button>
        </div>
      </>
    );
  }

  const cost = result.cost!;
  const { int: costInt, dec: costDec } = formatPrice(cost.cif_per_ton);
  const contractChecks = result.indicator_check.filter(isContractIndicator);
  const totalIndicators = contractChecks.length;
  const passing = contractChecks.filter(isIndicatorPassing).length;
  const qualityStatus = result.quality_status ?? "Estimated";
  const today = new Date();
  const dateStr = `${today.getFullYear()} 年 ${today.getMonth() + 1} 月 ${today.getDate()} 日`;

  const sortedRecipe = [...result.orders].sort((a, b) => b.ratio - a.ratio);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-subtitle">{dateStr}</div>
          <h1 className="page-title">今日配煤</h1>
        </div>
        <div className="contract-chip">{contractName || "默认合同"}</div>
      </div>

      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          alignItems: "center",
          gap: 6,
          fontSize: 12,
          color: "var(--c-text-3)",
          marginBottom: 12,
        }}
      >
        <span>基于</span>
        <button onClick={() => onNavigate("contract")} style={summaryLink}>
          {contractName || "默认合同"} ▸
        </button>
        <span>·</span>
        <button onClick={() => onNavigate("pool")} style={summaryLink}>
          参与求解 {enabledCount} 种煤 ▸
        </button>
        <span>· 采购</span>
        <input
          type="number"
          inputMode="numeric"
          value={qtyInput}
          onChange={(e) => {
            qtyInputRef.current = e.target.value;
            setQtyInput(e.target.value);
            setSaveMsg(null);
            setExportMsg(null);
          }}
          onBlur={commitQty}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
          aria-label="采购总吨数"
          style={{
            width: 64,
            padding: "2px 6px",
            borderRadius: 6,
            border: "1px solid var(--c-border, #d1d5db)",
            fontSize: 12,
            textAlign: "right",
            color: "var(--c-text-1, #111)",
            background: "var(--c-card, #fff)",
          }}
        />
        <span>吨</span>
      </div>

      {(refreshing || !actionsEnabled) && (
        <div
          role="status"
          style={{
            marginBottom: 12,
            padding: "8px 10px",
            borderRadius: 8,
            background: "#fef3c7",
            color: "#92400e",
            fontSize: 12,
          }}
        >
          {refreshing
            ? "输入已变更，正在重算；当前结果仅供参考，暂不可保存或导出。"
            : "采购量尚未与当前结果同步；离开输入框后会重新计算。"}
        </div>
      )}

      <div className="today-dashboard">
      <div className="cost-card today-cost">
        <div className="cost-label">最低到厂价</div>
        <div className="cost-amount">
          <span className="cost-int">{costInt}</span>
          <span className="cost-dec">.{costDec}</span>
          <span className="cost-unit">元/吨</span>
        </div>
        <div className="cost-meta">
          <span className="badge">
            {passing}/{totalIndicators} 项数值界内
          </span>
          <span className="badge">
            质量：{QUALITY_STATUS_LABEL[qualityStatus]}
          </span>
          <span style={{ opacity: 0.85 }}>
            {enabledCount} 种煤可选
          </span>
          {cost.total_cif != null && (
            <span style={{ opacity: 0.85 }}>
              总额 {Math.round(cost.total_cif).toLocaleString("zh-CN")} 元
            </span>
          )}
        </div>
        {drift && (
          <div className="cost-drift">
            按锚点推算 {drift.avgRatio >= 1 ? "+" : "−"}
            {(Math.abs(drift.avgRatio - 1) * 100).toFixed(1)}%
            {" · "}
            {drift.count} 种煤用旧报价推算
            {drift.oldestQuotedAt && ` · 最旧报价 ${drift.oldestQuotedAt}`}
            {drift.anchors.length > 0 && ` · 锚点 ${drift.anchors.join("、")}`}
          </div>
        )}
      </div>

      <div className="card today-recipe">
        <div
          className="card-title"
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
          }}
        >
          <span>今日配方</span>
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
              </div>
              <div className="recipe-amount">
                {(o.ratio * 100).toFixed(2)}% ·{" "}
                {o.tons != null ? `${o.tons.toFixed(0)} 吨` : "-"}
              </div>
            </div>
          ))}
        </div>
      </div>

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
                  <span
                    className={`evaluation-pill evaluation-${status.toLowerCase()}`}
                  >
                    {EVALUATION_STATUS_LABEL[status]}
                  </span>
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
              </div>
            );
          })}
        </div>
      </div>
      </div>

      {result.warnings.length > 0 && (
        <div
          className="binding-list"
          style={{
            background: "#fef3c7",
            borderColor: "#fde6b3",
            marginTop: 12,
          }}
        >
          <div className="binding-list-title">提示</div>
          {result.warnings.map((w, i) => (
            <div key={i}>{w}</div>
          ))}
        </div>
      )}

      <div className="action-row">
        <button
          className="btn btn-secondary"
          onClick={() => runSolve(false)}
          disabled={recompute === "running"}
        >
          {recompute === "running"
            ? "重算中..."
            : recompute === "done"
            ? "✓ 已重算"
            : "重新计算"}
        </button>
        <button
          className="btn btn-secondary"
          onClick={exportOrder}
          disabled={!actionsEnabled}
        >
          {exportMsg ?? "导出订单"}
        </button>
        <button
          className="btn btn-primary today-save"
          onClick={() => void saveToHistory()}
          disabled={!actionsEnabled || saving}
        >
          {saving ? "保存中..." : saveMsg ?? "保存方案"}
        </button>
      </div>
    </>
  );
}
