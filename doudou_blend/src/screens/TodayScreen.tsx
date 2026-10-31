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
import { resolveCoalPool, toEstimateCoal } from "../domain/resolvedCoal";
import {
  extractOrphanedGuarantees,
  purchaseTermsByCoalName,
  resolvePurchaseTerms,
} from "../domain/purchaseTerms";
import { hasCostAdjustments } from "../domain/costBreakdown";
import {
  isSnapshotActionable,
  LatestRequestTracker,
  type SolveSnapshot,
} from "../domain/solveSession";
import { loadMaster } from "../master_loader";
import { getPenaltyTemplate, PENALTY_TEMPLATE_EVENT } from "../penaltyStorage";
import type {
  BlendResult,
  Spec,
} from "../types";
import type { TabId } from "../TabBar";
import {
  getCoalPrefs,
  getQuantity,
  getShovelTons,
  getUserContract,
  getUserCoals,
  setQuantity,
} from "../storage";
import { estimateRequest, loadPriceFactor } from "../domain/blendRequest";
import { coalTypeMap } from "../domain/coalTypes";
import { CostCard } from "./CostCard";
import { QuoteScreen } from "./QuoteScreen";
import { RecipeCard, IndicatorCard, QUALITY_STATUS_LABEL, isContractIndicator, isIndicatorPassing } from "./BlendCards";
import { InfeasiblePanel } from "./InfeasiblePanel";

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
    const netPerTon = cost.net_per_ton ?? cost.cif_per_ton;
    // 实际成本才是真结算价; 跟到厂价没差别时不重复报两遍. 用共享判定
    // hasCostAdjustments, 不要自己拼 |net-cif|>eps —— 见 CostCard.tsx 同款注释:
    // 买入折扣与卖出扣款刚好抵消时 net===cif, 但两笔调整确实各自发生过.
    const priceLine = hasCostAdjustments(cost)
      ? `实际成本 ${netPerTon.toFixed(2)} 元/吨 (到厂价 ${cost.cif_per_ton.toFixed(2)})`
      : `到厂价 ${cost.cif_per_ton.toFixed(2)} 元/吨`;
    lines.push(`${priceLine} | 数值界内 ${passing}/${total} | 质量 ${quality}`);
  }
  lines.push("──────────────────");
  for (const o of [...result.orders].sort((a, b) => b.ratio - a.ratio)) {
    const pct = `${(o.ratio * 100).toFixed(1)}%`;
    const tons =
      (o.tons != null ? `${Math.round(o.tons)}吨` : "-") +
      (o.shovels != null ? ` ${o.shovels}铲` : "");
    // 结算按 cif_eff_amount (含买入侧修正); 老记录没有这字段才退回报价金额.
    const settled = o.cif_eff_amount ?? o.cif_amount;
    const amt = settled != null ? `  ${yuan(settled)}` : "";
    lines.push(`${o.coal}  ${pct}  ${tons}${amt}`);
  }
  // 总额同理优先 total_net (真实应付), 不能拿 total_cif (报价合计) 顶替.
  const total_amount = cost?.total_net ?? cost?.total_cif;
  if (total_amount != null) {
    lines.push(`总额 ${yuan(total_amount)}`);
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
  // 询价精算开着时记下进入那一刻的估价结果: 后台重算 (同步、改煤池) 失败或重算中都不能把精算页
  // 卸掉, 否则用户当场填的报价就丢了.
  const [quoting, setQuoting] = useState<BlendResult | null>(null);
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
    // 监听 prefs/contract/user_coals/一铲吨数/扣款模板 变化, 自动重算 (inline, 不整页 loading)
    const refresh = () => void runSolve(false);
    window.addEventListener("doudou:prefs_changed", refresh);
    window.addEventListener("doudou:contract_changed", refresh);
    window.addEventListener("doudou:user_coals_changed", refresh);
    window.addEventListener("doudou:shovel_changed", refresh);
    window.addEventListener(PENALTY_TEMPLATE_EVENT, refresh);
    return () => {
      mountedRef.current = false;
      window.removeEventListener("doudou:prefs_changed", refresh);
      window.removeEventListener("doudou:contract_changed", refresh);
      window.removeEventListener("doudou:user_coals_changed", refresh);
      window.removeEventListener("doudou:shovel_changed", refresh);
      window.removeEventListener(PENALTY_TEMPLATE_EVENT, refresh);
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

      const pool = resolveCoalPool(master.coals, userCoals, prefs);
      // 采购扣款模板只存本机 localStorage, 不跨设备同步 (见 penaltyStorage.ts);
      // 换设备后煤的保证值同步了但模板没有, 会悄悄漏算扣款, 必须提示用户.
      //
      // 只跑一次 resolvePurchaseTerms: 求解请求要挂的 purchase_terms 和
      // 孤儿保证值告警都从这一次扫描派生, 不要分两处各自调 mergePurchaseTerms
      // —— 两处各自算同一件事, 数据一变就可能悄悄分叉 (这个项目撞过很多次)。
      const purchaseTermsEntries = resolvePurchaseTerms(
        pool,
        prefs,
        getPenaltyTemplate(),
      );
      const termsByCoal = purchaseTermsByCoalName(purchaseTermsEntries);
      const orphanedGuarantees = extractOrphanedGuarantees(purchaseTermsEntries);
      // 单煤真实报价拿不到: 煤价一律按质量估算, 全部放进价格因子 extra_coals.
      const estimated = pool
        .map((resolved) => toEstimateCoal(resolved, termsByCoal.get(resolved.name)))
        .filter((coal) => coal != null);

      if (!master.price_factor || estimated.length === 0) {
        tracker.accept(requestId, () => {
          setState({
            status: "error",
            error: master.price_factor
              ? "没有可参与求解的煤。请在煤池启用煤种。"
              : "缺少估价参数 (price_factor), 无法估算煤价。",
          });
        });
        return;
      }

      const specs: Spec[] = userContract ?? master.default_contract.specs;
      const contractName = userContract
        ? "用户自定义合同"
        : master.default_contract.name;

      const priceFactor = await loadPriceFactor(master.price_factor);
      if (!tracker.isCurrent(requestId)) return;
      const request = estimateRequest({
        specs, totalQuantity, shovelTons: getShovelTons(), priceFactor,
      }, estimated);

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
            enabledCount: estimated.length,
            orphanedGuarantees,
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

  if (quoting) {
    return <QuoteScreen todayResult={quoting} onBack={() => setQuoting(null)} />;
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
  const refreshing = state.status === "refreshing";
  const actionsEnabled = isSnapshotActionable(
    snapshot,
    requestTrackerRef.current.currentRequestId,
    qtyInput,
  );
  // 不可行不再提前 return: 页头、输入摘要这些上下文与求解成败无关,
  // 而恰恰是用户排查不可行时唯一的抓手 —— 只把结果区换成诊断面板.
  const cost = result.ok ? result.cost ?? null : null;
  // 总额展示优先 total_net (真实应付合计); 没算扣款条款前两者相等.
  const totalCost = cost ? cost.total_net ?? cost.total_cif : null;
  const contractChecks = result.indicator_check.filter(isContractIndicator);
  const totalIndicators = contractChecks.length;
  const passing = contractChecks.filter(isIndicatorPassing).length;
  const qualityStatus = result.quality_status ?? "Estimated";
  const today = new Date();
  const dateStr = `${today.getFullYear()} 年 ${today.getMonth() + 1} 月 ${today.getDate()} 日`;

  const coalTypes = coalTypeMap(snapshot.request);

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
        {snapshot.request.shovel_tons != null && (
          <>
            <span>·</span>
            <button onClick={() => onNavigate("me")} style={summaryLink}>
              一铲 {snapshot.request.shovel_tons} 吨 ▸
            </button>
          </>
        )}
      </div>

      {/* 吨数未同步的提示只在有配方时有意义: isSnapshotActionable 对不可行结果一律
          返回 false, 不加 result.ok 这一半, 每次不可行都会多出一条"采购量尚未同步"
          的假提示 —— 吨数明明是同步的, 只是没算出配方. 重算中的提示则两种情况都要. */}
      {(refreshing || (result.ok && !actionsEnabled)) && (
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

      {cost ? (
      <div className="today-dashboard">
      <div className="today-cost">
        <CostCard cost={cost} orphanedGuarantees={snapshot.orphanedGuarantees ?? []} />
        <div className="cost-card cost-status-card">
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
            {totalCost != null && (
              <span style={{ opacity: 0.85 }}>
                总额 {Math.round(totalCost).toLocaleString("zh-CN")} 元
              </span>
            )}
          </div>
        </div>
      </div>

      <RecipeCard result={result} coalTypes={coalTypes} />
      <IndicatorCard result={result} />
      </div>
      ) : (
        <>
          <InfeasiblePanel result={result} />
          {/* 求解侧提示只在没解出来时给: 剔煤、岩相复核这类原因能解释为什么不可行.
              解出来时不显示 —— 今日页只留配方、成本、指标. */}
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
          {/* 只在真·不可行时给这条建议: ok 却没有成本结构是结果不完整, 放宽合同
              解决不了, 那时这句是错的指路. */}
          {!result.ok && (
            <p style={{ fontSize: 12, color: "var(--c-text-3)", marginTop: 12 }}>
              建议: 去「合同」放宽某项约束, 或去「煤池」启用更多煤源.
            </p>
          )}
        </>
      )}

      <div className="action-row">
        <button
          className="btn btn-secondary"
          disabled={refreshing}
          onClick={() => setQuoting(result)}
        >
          询价精算
        </button>
        <button
          className="btn btn-secondary"
          onClick={() => runSolve(false)}
          disabled={recompute === "running"}
        >
          {recompute === "running"
            ? "重算中..."
            : recompute === "done"
            ? "✓ 已重算"
            : result.ok
            ? "重新计算"
            : "重试"}
        </button>
        {/* 没有配方就没有可导出、可保存的东西, 这两个按钮只在成功时出现. */}
        {result.ok && (
          <>
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
          </>
        )}
      </div>
    </>
  );
}
