/**
 * 屏 1 - 今日（首页）
 *
 * 用 master 里 verified 状态的 4 主力煤 + 默认合同, 跑 LP 求解.
 * 展示成本、配方、8 项指标体检、binding 谈判方向.
 */
import { useEffect, useState } from "react";
import { getBackend } from "../backend";
import { loadMaster } from "../master_loader";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "../types";
import type { BlendRequest, BlendResult, MasterCoalEntry } from "../types";

const RECIPE_COLORS = ["#0a5fff", "#7c3aed", "#ec4899", "#f59e0b", "#10b981"];

function formatPrice(n: number): { int: string; dec: string } {
  const [intPart, decPart] = n.toFixed(2).split(".");
  return { int: intPart, dec: decPart };
}

interface SolveState {
  status: "loading" | "ok" | "error";
  result?: BlendResult;
  contractName?: string;
  error?: string;
}

export function TodayScreen() {
  const [state, setState] = useState<SolveState>({ status: "loading" });

  useEffect(() => {
    void runSolve();
  }, []);

  async function runSolve() {
    setState({ status: "loading" });
    try {
      const master = await loadMaster();
      const verified: MasterCoalEntry[] = master.coals.filter(
        (c) => c.status === "verified"
      );
      if (verified.length === 0) {
        setState({ status: "error", error: "没有 verified 主力煤" });
        return;
      }

      const request: BlendRequest = {
        coals: verified.map((c) => ({
          name: c.name,
          props: c.props,
          fob: c.fob!,
          frt: c.frt!,
        })),
        specs: master.default_contract.specs,
        total_quantity: 3700,
        truncate_decimal: true,
      };

      const backend = await getBackend();
      const json = await backend.solveJson(JSON.stringify(request));
      const result: BlendResult = JSON.parse(json);
      setState({
        status: "ok",
        result,
        contractName: master.default_contract.name,
      });
    } catch (e) {
      setState({ status: "error", error: String(e) });
    }
  }

  if (state.status === "loading") {
    return <div className="loading">求解中...</div>;
  }
  if (state.status === "error" || !state.result) {
    return (
      <div className="empty">
        <p>出错了: {state.error}</p>
        <button className="btn btn-primary" onClick={runSolve}>
          重试
        </button>
      </div>
    );
  }

  const { result, contractName } = state;
  if (!result.ok) {
    return (
      <div className="empty">
        <p>当前配置不可行: {result.reason}</p>
        <button className="btn btn-primary" onClick={runSolve}>
          重试
        </button>
      </div>
    );
  }

  const cost = result.cost!;
  const { int: costInt, dec: costDec } = formatPrice(cost.cif_per_ton);
  const totalIndicators = result.indicator_check.length;
  const passing = result.indicator_check.filter(
    (ic) => !ic.slack || ic.slack >= -0.01
  ).length;
  const today = new Date();
  const dateStr = `${today.getFullYear()} 年 ${today.getMonth() + 1} 月 ${today.getDate()} 日`;

  const sortedRecipe = [...result.orders].sort((a, b) => b.ratio - a.ratio);
  const binding = result.indicator_check.filter((ic) => ic.binding);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-subtitle">{dateStr}</div>
          <h1 className="page-title">今日配煤</h1>
        </div>
        <div className="contract-chip">{contractName || "默认合同"}</div>
      </div>

      {/* 成本卡 */}
      <div className="cost-card">
        <div className="cost-label">最低到厂价</div>
        <div className="cost-amount">
          <span className="cost-int">{costInt}</span>
          <span className="cost-dec">.{costDec}</span>
          <span className="cost-unit">元/吨</span>
        </div>
        <div className="cost-meta">
          <span className="badge">
            {passing}/{totalIndicators} 项达标
          </span>
          {cost.total_cif != null && (
            <span style={{ opacity: 0.85 }}>
              总额 {cost.total_cif.toLocaleString("zh-CN", { maximumFractionDigits: 0 })} 元
            </span>
          )}
        </div>
      </div>

      {/* 配方卡 */}
      <div className="card">
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
            共 {sortedRecipe.length} 种煤
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

      {/* 8 项指标 */}
      <div className="card">
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
            vs {contractName}
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
            const violated = ic.slack != null && ic.slack < -0.01;
            const className = `indicator-cell ${
              ic.binding ? "binding" : violated ? "violated" : ""
            }`;
            let rangeStr = "—";
            if (ic.min != null && ic.max != null)
              rangeStr = `${ic.min}-${ic.max}`;
            else if (ic.min != null) rangeStr = `≥${ic.min}`;
            else if (ic.max != null) rangeStr = `≤${ic.max}`;
            return (
              <div key={key} className={className}>
                <div className="indicator-label">{label}</div>
                <div className="indicator-value">{ic.value.toFixed(2)}</div>
                <div className="indicator-meta">
                  {rangeStr} {ic.binding ? "★ 顶格" : violated ? "✗" : "✓"}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* 谈判方向 */}
      {binding.length > 0 && (
        <div className="binding-list">
          <div className="binding-list-title">谈判方向 (binding 顶格)</div>
          {binding.map((ic) => (
            <div key={ic.indicator}>
              {ic.label_zh} 顶格 → 找{ic.max != null ? "更低" : "更高"}
              {ic.label_zh}的煤源, 或谈宽合同
            </div>
          ))}
        </div>
      )}

      {/* 警告 */}
      {result.warnings.length > 0 && (
        <div className="binding-list" style={{ background: "#fef3c7", borderColor: "#fde6b3", marginTop: 12 }}>
          <div className="binding-list-title">提示</div>
          {result.warnings.map((w, i) => (
            <div key={i}>{w}</div>
          ))}
        </div>
      )}

      <div className="action-row">
        <button className="btn btn-secondary" onClick={runSolve}>
          重新计算
        </button>
        <button className="btn btn-primary" onClick={() => alert("方案保存到历史 (TODO)")}>
          保存方案
        </button>
      </div>
    </>
  );
}
