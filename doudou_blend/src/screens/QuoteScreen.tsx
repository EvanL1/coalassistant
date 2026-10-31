/**
 * 询价精算: 用户当场填几种煤的出厂价与运费 (可改这批指标), 只用这几种煤求解.
 *
 * 询价当场有效: 不写煤池、偏好或本地存储, 下次进来输入框为空 (上次询价只作占位提示).
 * 每次精算按批存档 (同一会话同一 batchId, 服务端以最后一次为准), 估价由服务端重算.
 */
import { useEffect, useRef, useState } from "react";
import { getBackend } from "../backend";
import { loadMaster } from "../master_loader";
import { getCoalPrefs, getQuantity, getShovelTons, getUserCoals, getUserContract } from "../storage";
import { getPenaltyTemplate } from "../penaltyStorage";
import { archiveQuotes, fetchLatestQuotes, type QuoteItemInput } from "../quotes";
import { resolveCoalPool, type ResolvedCoal } from "../domain/resolvedCoal";
import { loadPriceFactor, quoteRequest, type QuotedCoal } from "../domain/blendRequest";
import {
  extractOrphanedGuarantees,
  purchaseTermsByCoalName,
  resolvePurchaseTerms,
} from "../domain/purchaseTerms";
import { LatestRequestTracker, type SolveSnapshot } from "../domain/solveSession";
import { coalTypeMap } from "../domain/coalTypes";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "../types";
import type { BlendResult, CoalMaster, CoalValue, IndicatorKey, LatestQuote } from "../types";
import { CostCard } from "./CostCard";
import { IndicatorCard, RecipeCard } from "./BlendCards";
import { InfeasiblePanel } from "./InfeasiblePanel";

/** 报价偏离今日估价超过这个比例就提示核对 (防 2150 打成 21500), 不拦. */
const QUOTE_DEVIATION_WARN = 0.3;
/** 与服务端 quotes.rs 的上限一致. */
const MAX_FOB = 100_000;
const MAX_FRT = 10_000;

interface QuoteRow {
  coal: ResolvedCoal;
  fob: string;
  frt: string;
  /** 当场改的指标原文; 空或非数字 = 用煤池值. */
  props: Partial<Record<IndicatorKey, string>>;
  expanded: boolean;
}

const emptyRow = (coal: ResolvedCoal): QuoteRow => ({
  coal,
  fob: "",
  frt: "",
  props: {},
  expanded: false,
});

const finitePositive = (value: number | null | undefined): value is number =>
  typeof value === "number" && Number.isFinite(value) && value > 0;

const fobInvalid = (text: string) => {
  const fob = Number(text);
  return text.trim() !== "" && (!finitePositive(fob) || fob >= MAX_FOB);
};

const frtInvalid = (text: string) => {
  const frt = Number(text);
  return text.trim() !== "" && (!Number.isFinite(frt) || frt < 0 || frt >= MAX_FRT);
};

/** 与煤池值不同的有效改动. */
function propEdits(row: QuoteRow): Partial<Record<IndicatorKey, number>> {
  const changed: Partial<Record<IndicatorKey, number>> = {};
  for (const key of INDICATOR_ORDER) {
    const text = row.props[key]?.trim();
    if (!text) continue;
    const value = Number(text);
    if (Number.isFinite(value) && value >= 0 && value !== row.coal.props[key]) {
      changed[key] = value;
    }
  }
  return changed;
}

function placeholder(quote: LatestQuote | undefined, field: "fob" | "frt"): string {
  if (quote) return `上次 ${quote[field]} (${quote.quoted_on.slice(5)})`;
  return field === "frt" ? "不填记 0" : "出厂价";
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function QuoteScreen({
  todayResult,
  onBack,
}: {
  todayResult: BlendResult;
  onBack: () => void;
}) {
  const [batchId] = useState(() => crypto.randomUUID());
  const [data, setData] = useState<{ master: CoalMaster; pool: ResolvedCoal[] } | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [rows, setRows] = useState<QuoteRow[]>([]);
  const [latest, setLatest] = useState(new Map<string, LatestQuote>());
  const [snapshot, setSnapshot] = useState<SolveSnapshot | null>(null);
  const [solving, setSolving] = useState(false);
  const [solveError, setSolveError] = useState<string | null>(null);
  const [archiveError, setArchiveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);
  const tracker = useRef(new LatestRequestTracker());
  const archiveQueue = useRef<Promise<unknown>>(Promise.resolve());
  const savingRef = useRef(false);

  useEffect(() => {
    let active = true;
    const prefs = getCoalPrefs();
    const userCoals = getUserCoals();
    void loadMaster()
      .then((master) => {
        if (!active) return;
        const pool = resolveCoalPool(master.coals, userCoals, prefs).filter(
          (coal) => coal.effectiveEnabled && coal.readiness === "ready",
        );
        setData({ master, pool });
        // 默认列出今日估价配方里的煤 (估价求解本身就是粗筛), 按用量排序.
        setRows(
          [...todayResult.orders]
            .sort((a, b) => b.ratio - a.ratio)
            .flatMap((order) => {
              const coal = pool.find((coal) => coal.name === order.coal);
              return coal ? [emptyRow(coal)] : [];
            }),
        );
      })
      .catch((error: unknown) => {
        if (active) setLoadError(String(error));
      });
    void fetchLatestQuotes().then((quotes) => {
      if (active) setLatest(quotes);
    });
    return () => {
      active = false;
      tracker.current.invalidate();
    };
  }, []);

  /** 改了输入, 旧结果就不再对应当前报价: 作废在途求解并清掉结果. */
  function changeRows(update: (previous: QuoteRow[]) => QuoteRow[]) {
    tracker.current.invalidate();
    setRows(update);
    setSnapshot(null);
    setSolving(false);
    setSolveError(null);
    setArchiveError(null);
    setSaveMsg(null);
  }

  function edit(name: string, patch: Partial<QuoteRow>) {
    changeRows((previous) =>
      previous.map((row) => (row.coal.name === name ? { ...row, ...patch } : row)),
    );
  }

  function toggle(name: string) {
    setRows((previous) =>
      previous.map((row) =>
        row.coal.name === name ? { ...row, expanded: !row.expanded } : row,
      ),
    );
  }

  const pricedRows = rows.filter((row) => finitePositive(Number(row.fob)));
  const invalidPrice = pricedRows.some((row) => fobInvalid(row.fob) || frtInvalid(row.frt));
  const canSolve = data?.master.price_factor && pricedRows.length >= 2 && !invalidPrice;

  async function runSolve() {
    if (!canSolve || !data?.master.price_factor) return;
    const requestId = tracker.current.issue();
    // 现场输入、合同和条款在任何 await 前冻结; 求解与存档使用同一批指标.
    const totalQuantity = getQuantity();
    const shovelTons = getShovelTons();
    const contract = getUserContract();
    const specs = contract ?? data.master.default_contract.specs;
    const contractName = contract ? "用户自定义合同" : data.master.default_contract.name;
    const terms = resolvePurchaseTerms(
      pricedRows.map((row) => row.coal),
      getCoalPrefs(),
      getPenaltyTemplate(),
    );
    const termsByName = purchaseTermsByCoalName(terms);
    const quoted: QuotedCoal[] = pricedRows.map((row) => ({
      coal: row.coal,
      fob: Number(row.fob),
      frt: Number(row.frt),
      props: propEdits(row),
      purchaseTerms: termsByName.get(row.coal.name),
    }));
    const items: QuoteItemInput[] = quoted.map((item) => ({
      name: item.coal.name,
      fob: item.fob,
      frt: item.frt,
      coal_type: item.coal.coal_type ?? undefined,
      form: item.coal.form ?? undefined,
      region: item.coal.region ?? undefined,
      props: { ...item.coal.props, ...item.props },
      changed: Object.keys(item.props) as IndicatorKey[],
    }));
    setSolving(true);
    setSnapshot(null);
    setSolveError(null);
    setArchiveError(null);
    setSaveMsg(null);
    // 同批存档按点击顺序发送, 防止慢的旧请求最后覆盖新报价; 不等待求解, 也不阻塞结果.
    const archive = archiveQueue.current
      .catch(() => undefined)
      .then(() => archiveQuotes(batchId, items));
    archiveQueue.current = archive;
    void archive.catch((error: unknown) => {
      tracker.current.accept(requestId, () => setArchiveError(errorText(error)));
    });
    try {
      const [priceFactor, backend] = await Promise.all([
        loadPriceFactor(data.master.price_factor),
        getBackend(),
      ]);
      if (!tracker.current.isCurrent(requestId)) return;
      const request = quoteRequest({ specs, totalQuantity, shovelTons, priceFactor }, quoted);
      const result = JSON.parse(await backend.solveJson(JSON.stringify(request))) as BlendResult;
      tracker.current.accept(requestId, () =>
        setSnapshot({
          requestId,
          request,
          result,
          contractName,
          enabledCount: quoted.length,
          orphanedGuarantees: extractOrphanedGuarantees(terms),
        }),
      );
    } catch (error) {
      tracker.current.accept(requestId, () => setSolveError(errorText(error)));
    } finally {
      tracker.current.accept(requestId, () => setSolving(false));
    }
  }

  async function saveToHistory() {
    if (!snapshot?.result.ok || !snapshot.result.cost || savingRef.current) return;
    const current = snapshot;
    savingRef.current = true;
    setSaving(true);
    setSaveMsg(null);
    try {
      const backend = await getBackend();
      if (!tracker.current.isCurrent(current.requestId)) return;
      await backend.saveHistory(
        current.result,
        current.contractName,
        current.request.total_quantity!,
      );
      tracker.current.accept(current.requestId, () => setSaveMsg("✓ 已保存"));
    } catch {
      tracker.current.accept(current.requestId, () => setSaveMsg("保存失败"));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  }

  const inRecipe = new Set(todayResult.orders.map((order) => order.coal));
  return (
    <>
      <button className="btn btn-secondary" onClick={onBack}>
        ← 返回今日
      </button>
      <div className="page-header">
        <h1 className="page-title">询价精算</h1>
      </div>
      {loadError && <div role="alert">煤池加载失败: {loadError}</div>}
      {!data && !loadError && <div className="loading">加载煤池...</div>}
      {data && (
        <>
          <div className="quote-list">
            {rows.map((row) => (
              <QuoteRowCard
                key={row.coal.name}
                row={row}
                today={todayResult.coal_values?.find((value) => value.name === row.coal.name)}
                inRecipe={inRecipe.has(row.coal.name)}
                latest={latest.get(row.coal.name)}
                onEdit={(patch) => edit(row.coal.name, patch)}
                onRemove={() =>
                  changeRows((previous) =>
                    previous.filter((item) => item.coal.name !== row.coal.name),
                  )
                }
                onToggle={() => toggle(row.coal.name)}
              />
            ))}
          </div>
          <select
            className="quote-add"
            aria-label="添加煤"
            value=""
            onChange={(e) => {
              const coal = data.pool.find((coal) => coal.name === e.target.value);
              if (coal) changeRows((previous) => [...previous, emptyRow(coal)]);
            }}
          >
            <option value="">添加煤</option>
            {data.pool
              .filter((coal) => !rows.some((row) => row.coal.name === coal.name))
              .map((coal) => (
                <option key={coal.name} value={coal.name}>
                  {coal.name}
                </option>
              ))}
          </select>
          {!data.master.price_factor && (
            <p role="alert">缺少估价参数 (price_factor), 无法精算</p>
          )}
          {pricedRows.length < 2 && <p className="quote-hint">至少填两种煤的出厂价</p>}
          <div className="action-row">
            <button
              className="btn btn-primary"
              disabled={!canSolve || solving}
              onClick={() => void runSolve()}
            >
              {solving ? "精算中..." : "精算"}
            </button>
          </div>
        </>
      )}
      {solveError && <p role="alert">精算失败: {solveError}</p>}
      {archiveError && (
        <p role="status" className="quote-warning">
          询价未存档: {archiveError}
        </p>
      )}
      {snapshot &&
        (snapshot.result.ok && snapshot.result.cost ? (
          <>
            <div className="today-dashboard quote-result">
              <div className="today-cost">
                <CostCard
                  cost={snapshot.result.cost}
                  orphanedGuarantees={snapshot.orphanedGuarantees}
                />
              </div>
              <RecipeCard
                result={snapshot.result}
                coalTypes={coalTypeMap(snapshot.request)}
                title="精算配方"
              />
              <IndicatorCard result={snapshot.result} />
            </div>
            <QuoteComparison snapshot={snapshot} />
            <div className="action-row">
              <button
                className="btn btn-primary"
                disabled={saving}
                onClick={() => void saveToHistory()}
              >
                {saving ? "保存中..." : saveMsg ?? "保存到历史"}
              </button>
            </div>
          </>
        ) : (
          <>
            <InfeasiblePanel result={snapshot.result} />
            {snapshot.result.warnings.length > 0 && (
              <div className="binding-list quote-warning">
                <div className="binding-list-title">提示</div>
                {snapshot.result.warnings.map((warning, index) => (
                  <div key={index}>{warning}</div>
                ))}
              </div>
            )}
          </>
        ))}
    </>
  );
}

/** 一种煤的询价: 出厂价、运费, 可展开改这批指标. */
function QuoteRowCard({
  row,
  today,
  inRecipe,
  latest,
  onEdit,
  onRemove,
  onToggle,
}: {
  row: QuoteRow;
  /** 今日估价结果里这种煤的估价与配煤价值. */
  today: CoalValue | undefined;
  inRecipe: boolean;
  latest: LatestQuote | undefined;
  onEdit: (patch: Partial<QuoteRow>) => void;
  onRemove: () => void;
  onToggle: () => void;
}) {
  const name = row.coal.name;
  const fob = Number(row.fob);
  const deviation =
    finitePositive(today?.fair) && finitePositive(fob) ? (fob - today.fair) / today.fair : 0;
  const changed = propEdits(row);
  return (
    <section className="card quote-row" aria-label={`询价 ${name}`}>
      <div className="quote-heading">
        <strong>{name}</strong>
        <span>今日估价 {finitePositive(today?.fair) ? Math.round(today.fair) : "—"} 元</span>
        <button className="btn btn-secondary" aria-label={`去掉 ${name}`} onClick={onRemove}>
          去掉
        </button>
      </div>
      <div className="quote-prices">
        <label>
          出厂价
          <input
            inputMode="decimal"
            aria-label={`${name} 出厂价`}
            value={row.fob}
            placeholder={placeholder(latest, "fob")}
            onChange={(e) => onEdit({ fob: e.target.value })}
          />
        </label>
        <label>
          运费
          <input
            inputMode="decimal"
            aria-label={`${name} 运费`}
            value={row.frt}
            placeholder={placeholder(latest, "frt")}
            onChange={(e) => onEdit({ frt: e.target.value })}
          />
        </label>
      </div>
      {fobInvalid(row.fob) && <p className="quote-warning">出厂价须在 0–100000 元之间</p>}
      {frtInvalid(row.frt) && <p className="quote-warning">运费须在 0–10000 元之间, 可填 0</p>}
      {/* 已入选的煤, 对偶给出的就是它的现价, 显示出来没有信息量. */}
      {!inRecipe && finitePositive(today?.blend_value) && (
        <p className="quote-hint">到厂价低于 {Math.round(today.blend_value)} 元才会配进去</p>
      )}
      {Math.abs(deviation) > QUOTE_DEVIATION_WARN && (
        <p className="quote-warning">
          比今日估价{deviation > 0 ? "高" : "低"} {Math.round(Math.abs(deviation) * 100)}%, 请核对
        </p>
      )}
      <button
        className="btn btn-secondary"
        aria-label={`改这批指标 ${name}`}
        aria-expanded={row.expanded}
        onClick={onToggle}
      >
        改这批指标
      </button>
      {row.expanded && (
        <div className="quote-props">
          {INDICATOR_ORDER.map((key) => {
            const text = row.props[key];
            const invalid =
              text != null &&
              text.trim() !== "" &&
              (!Number.isFinite(Number(text)) || Number(text) < 0);
            return (
              <label key={key}>
                {INDICATOR_LABEL[key]}{" "}
                {changed[key] != null && <span className="quote-changed">{key} 已改</span>}
                <input
                  inputMode="decimal"
                  aria-label={`${name} ${key}`}
                  value={text ?? String(row.coal.props[key] ?? "")}
                  onChange={(e) => onEdit({ props: { ...row.props, [key]: e.target.value } })}
                />
                {invalid && <span className="quote-warning">{key} 不是有效数字, 沿用煤池值</span>}
              </label>
            );
          })}
        </div>
      )}
    </section>
  );
}

/** 本批煤的报价与估价对比 (只列这批的煤). */
function QuoteComparison({ snapshot }: { snapshot: SolveSnapshot }) {
  return (
    <div className="card quote-comparison">
      <h2 className="card-title">报价对比</h2>
      <table aria-label="报价对比">
        <thead>
          <tr>
            <th>煤</th>
            <th>用量</th>
            <th>出厂报价</th>
            <th>估价</th>
            <th>差额</th>
          </tr>
        </thead>
        <tbody>
          {snapshot.request.coals.map((coal) => {
            const value = snapshot.result.coal_values?.find((value) => value.name === coal.name);
            const order = snapshot.result.orders.find((order) => order.coal === coal.name);
            // 无基准时核心会从本批报价反推价格, 不能拿它评价报价.
            const fair = value?.benchmark ? value.fair : null;
            const gap = value?.benchmark ? value.gap : null;
            return (
              <tr key={coal.name}>
                <td>{coal.name}</td>
                <td>{Math.round(order?.tons ?? 0)} 吨</td>
                <td>{finitePositive(value?.fob) ? Math.round(value.fob) : "—"}</td>
                <td>{finitePositive(fair) ? Math.round(fair) : "—"}</td>
                <td>{formatGap(gap)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function formatGap(gap: number | null | undefined): string {
  if (gap == null || !Number.isFinite(gap)) return "—";
  if (gap === 0) return "持平";
  return `${gap > 0 ? "高" : "低"} ${Math.round(Math.abs(gap))} 元`;
}
