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
import { useEffect, useState, type CSSProperties } from "react";
import { getBackend } from "../backend";
import { loadMaster } from "../master_loader";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "../types";
import type {
  BlendRequest,
  BlendResult,
  Coal,
  MasterCoalEntry,
  Spec,
} from "../types";
import type { TabId } from "../TabBar";
import {
  appendHistory,
  getCoalPrefs,
  getQuantity,
  getUserContract,
  getUserCoals,
  setQuantity,
  type CoalPrefs,
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

/** 把求解结果格式化成可复制的采购清单文本 (纯函数). */
function buildOrderText(
  result: BlendResult,
  contractName: string,
  isoDate: string,
  quantity: number,
): string {
  const cost = result.cost;
  const total = result.indicator_check.length;
  const passing = result.indicator_check.filter(
    (ic) => ic.slack == null || ic.slack >= -0.01,
  ).length;
  const yuan = (n: number) => `¥${Math.round(n).toLocaleString("zh-CN")}`;

  const lines: string[] = [];
  lines.push(`【豆哥配煤】${isoDate}`);
  lines.push(`合同: ${contractName || "默认合同"} | 总量 ${quantity} 吨`);
  if (cost) {
    lines.push(`到厂价 ${cost.cif_per_ton.toFixed(2)} 元/吨 | 达标 ${passing}/${total}`);
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

/** 应用 user_overrides 到 master 煤上, 返回 LP 用的 Coal. */
function applyOverrides(coal: MasterCoalEntry, prefs: CoalPrefs): Coal | null {
  const pref = prefs[coal.name];
  // 基础检查: 必须有 fob/frt
  const fob = pref?.fob_override ?? coal.fob;
  const frt = pref?.frt_override ?? coal.frt;
  if (fob == null || frt == null) return null;

  // 合并 props: master + override
  const props: Record<string, number> = {};
  for (const [k, v] of Object.entries(coal.props)) {
    if (v != null) props[k] = v;
  }
  if (pref?.props_override) {
    for (const [k, v] of Object.entries(pref.props_override)) {
      if (v != null) props[k] = v;
    }
  }

  return { name: coal.name, props, fob, frt };
}

/** 启用状态: hidden 一票否决, 否则 显式 override > master verified 默认启用. */
function isEnabled(coal: MasterCoalEntry, prefs: CoalPrefs): boolean {
  const p = prefs[coal.name];
  if (p?.hidden) return false;
  if (p?.enabled != null) return p.enabled;
  return coal.status === "verified";
}

interface SolveState {
  status: "loading" | "ok" | "error";
  result?: BlendResult;
  contractName?: string;
  request?: BlendRequest;
  enabledCount?: number;
  error?: string;
}

export function TodayScreen({ onNavigate }: { onNavigate: (tab: TabId) => void }) {
  const [state, setState] = useState<SolveState>({ status: "loading" });
  const [saveFlag, setSaveFlag] = useState(false);
  // 重算反馈: idle / running / done. running 时按钮显示"重算中...", done 时显示"✓ 已重算" 1.5s
  const [recompute, setRecompute] = useState<"idle" | "running" | "done">("idle");
  // 采购吨数输入 (字符串以便编辑); 导出反馈文案
  const [qtyInput, setQtyInput] = useState(() => String(getQuantity()));
  const [exportMsg, setExportMsg] = useState<string | null>(null);

  useEffect(() => {
    void runSolve(true);
    // 监听 prefs/contract/user_coals 变化, 自动重算 (inline, 不整页 loading)
    const refresh = () => void runSolve(false);
    window.addEventListener("doudou:prefs_changed", refresh);
    window.addEventListener("doudou:contract_changed", refresh);
    window.addEventListener("doudou:user_coals_changed", refresh);
    return () => {
      window.removeEventListener("doudou:prefs_changed", refresh);
      window.removeEventListener("doudou:contract_changed", refresh);
      window.removeEventListener("doudou:user_coals_changed", refresh);
    };
  }, []);

  /** initial=true 时整页 loading; 否则原地反馈, 保留旧结果直到新结果出来. */
  async function runSolve(initial: boolean = false) {
    if (initial) setState({ status: "loading" });
    else setRecompute("running");
    try {
      const master = await loadMaster();
      const prefs = getCoalPrefs();
      const userContract = getUserContract();
      const userCoals = getUserCoals();

      // 启用煤集: master + 用户新增 (新增的也走同样的启用判定 + override 流程)
      // 没填 fob/frt 的会在 applyOverrides 里被过滤掉 (返回 null), 所以
      // 用户刚新增、还没在 CoalEditor 里补价格的 draft 煤自动不会被卷进 LP.
      const enabledAll = [...master.coals, ...userCoals].filter((c) =>
        isEnabled(c, prefs),
      );
      const coals = enabledAll
        .map((c) => applyOverrides(c, prefs))
        .filter((c): c is Coal => c != null);

      if (coals.length === 0) {
        setState({
          status: "error",
          error: "没有启用的煤. 去煤池启用一些主力煤吧.",
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
        total_quantity: getQuantity(),
        truncate_decimal: true,
      };

      const backend = await getBackend();
      const json = await backend.solveJson(JSON.stringify(request));
      const result: BlendResult = JSON.parse(json);
      setState({
        status: "ok",
        result,
        contractName,
        request,
        enabledCount: coals.length,
      });
    } catch (e) {
      setState({ status: "error", error: String(e) });
    } finally {
      if (!initial) {
        setRecompute("done");
        setTimeout(() => setRecompute("idle"), 1500);
      }
    }
  }

  function saveToHistory() {
    if (state.status !== "ok" || !state.result?.cost || !state.result.ok) return;
    const recipe: Record<string, number> = {};
    for (const o of state.result.orders) recipe[o.coal] = o.ratio;
    appendHistory({
      cost_cif: state.result.cost.cif_per_ton,
      recipe,
      contract_name: state.contractName || "",
    });
    setSaveFlag(true);
    setTimeout(() => setSaveFlag(false), 2000);
  }

  /** 提交采购吨数: 合法则持久化并重算, 非法则回退上次值. */
  function commitQty() {
    const n = Number(qtyInput);
    if (Number.isFinite(n) && n > 0) {
      setQuantity(n);
      setQtyInput(String(n));
      void runSolve(false);
    } else {
      setQtyInput(String(getQuantity()));
    }
  }

  /** 导出: 把当前方案复制成采购清单文本. */
  async function exportOrder() {
    if (state.status !== "ok" || !state.result?.ok) return;
    const d = new Date();
    const iso = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    const text = buildOrderText(state.result, state.contractName ?? "", iso, getQuantity());
    const ok = await copyText(text);
    setExportMsg(ok ? "✓ 已复制清单" : "复制失败");
    setTimeout(() => setExportMsg(null), 2000);
  }

  if (state.status === "loading") {
    return <div className="loading">求解中...</div>;
  }
  if (state.status === "error" || !state.result) {
    return (
      <div className="empty">
        <p style={{ marginBottom: 16 }}>{state.error}</p>
        <button className="btn btn-primary" onClick={() => runSolve(true)}>
          重试
        </button>
      </div>
    );
  }

  const { result, contractName, enabledCount } = state;
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
          <button className="btn btn-secondary" onClick={() => runSolve(false)}>
            {recompute === "running" ? "重算中..." : recompute === "done" ? "✓ 已重算" : "重试"}
          </button>
        </div>
      </>
    );
  }

  const cost = result.cost!;
  const { int: costInt, dec: costDec } = formatPrice(cost.cif_per_ton);
  const totalIndicators = result.indicator_check.length;
  const passing = result.indicator_check.filter(
    (ic) => ic.slack == null || ic.slack >= -0.01
  ).length;
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
          启用 {enabledCount} 种煤 ▸
        </button>
        <span>· 采购</span>
        <input
          type="number"
          inputMode="numeric"
          value={qtyInput}
          onChange={(e) => setQtyInput(e.target.value)}
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
          <span style={{ opacity: 0.85 }}>
            {enabledCount} 种煤可选
          </span>
          {cost.total_cif != null && (
            <span style={{ opacity: 0.85 }}>
              总额 {Math.round(cost.total_cif).toLocaleString("zh-CN")} 元
            </span>
          )}
        </div>
      </div>

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
        <button className="btn btn-secondary" onClick={exportOrder}>
          {exportMsg ?? "导出订单"}
        </button>
        <button
          className="btn btn-primary"
          onClick={saveToHistory}
          style={{ gridColumn: "1 / -1" }}
        >
          {saveFlag ? "✓ 已保存" : "保存方案"}
        </button>
      </div>
    </>
  );
}
