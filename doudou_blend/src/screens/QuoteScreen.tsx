/**
 * 询价精算: 用户当场填几种煤的出厂价与运费 (可改这批指标), 只用这几种煤求解.
 *
 * 询价当场有效: 不写煤池、偏好或本地存储, 下次进来输入框为空 (上次询价只作占位提示).
 * 例外是会话内草稿 (sessionStorage): 没点精算就切走页签, 回来能接着填; 精算或返回即清掉.
 * 精算只给客户算配方, 不提供保存; 询价的存档是后台自动的, 客户不用操作.
 * 每次精算按批存档 (同一会话同一 batchId, 服务端以最后一次为准), 估价由服务端重算.
 */
import { useEffect, useRef, useState } from "react";
import { getBackend } from "../backend";
import { loadMaster } from "../master_loader";
import { getCoalPrefs, getQuantity, getShovelTons, getUserCoals, getUserContract } from "../storage";
import { getPenaltyTemplate } from "../penaltyStorage";
import { archiveQuotes, fetchLatestQuotes, type QuoteItemInput } from "../quotes";
import { resolveCoalPool, type ResolvedCoal } from "../domain/resolvedCoal";
import { loadPriceFactorWithStatus, quoteRequest, type QuotedCoal } from "../domain/blendRequest";
import { isPartialNumberInput, parseNumberInput } from "../domain/numberInput";
import { clearQuoteDraft, loadQuoteDraft, saveQuoteDraft, type QuoteDraftRow } from "../domain/quoteDraft";
import { isContractIncomplete } from "../domain/contract";
import { friendlySolverText } from "../domain/resultNotes";
import { errorMessage } from "../http";
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
  /** 当场改的指标原文; 空 = 用煤池值. */
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

/** 出厂价: 空 = 没报价; 认不出或越界给出错误. */
function fobField(text: string): { value: number | null; error: string | null } {
  const parsed = parseNumberInput(text, "price");
  if (!parsed.ok) return { value: null, error: parsed.error };
  if (parsed.value == null) return { value: null, error: null };
  return finitePositive(parsed.value) && parsed.value < MAX_FOB
    ? { value: parsed.value, error: null }
    : { value: null, error: "出厂价须在 0–100000 元之间" };
}

/** 运费: 空按 0 计. */
function frtField(text: string): { value: number | null; error: string | null } {
  const parsed = parseNumberInput(text, "price");
  if (!parsed.ok) return { value: null, error: parsed.error };
  if (parsed.value == null) return { value: null, error: null };
  return parsed.value >= 0 && parsed.value < MAX_FRT
    ? { value: parsed.value, error: null }
    : { value: null, error: "运费须在 0–10000 元之间, 可填 0" };
}

function propField(text: string | undefined): { value: number | null; error: string | null } {
  const parsed = parseNumberInput(text ?? "", "indicator");
  if (!parsed.ok) return { value: null, error: parsed.error };
  if (parsed.value != null && parsed.value < 0) return { value: null, error: "指标不能是负数" };
  return { value: parsed.value, error: null };
}

interface RowFieldError {
  /** 失焦跟踪用的键, 与输入框一一对应. */
  field: string;
  /** 报错里给人看的名字 ("甲 出厂价"、"甲 灰"). */
  label: string;
  raw: string;
  error: string;
}

/** 这一行填错的字段, 有一处就不能精算. */
function rowFieldErrors(row: QuoteRow): RowFieldError[] {
  const name = row.coal.name;
  const fields: (Omit<RowFieldError, "error"> & { error: string | null })[] = [
    { field: `${name}:fob`, label: `${name} 出厂价`, raw: row.fob, error: fobField(row.fob).error },
    { field: `${name}:frt`, label: `${name} 运费`, raw: row.frt, error: frtField(row.frt).error },
    ...INDICATOR_ORDER.map((key) => ({
      field: `${name}:prop:${key}`,
      label: `${name} ${INDICATOR_LABEL[key]}`,
      raw: row.props[key] ?? "",
      error: propField(row.props[key]).error,
    })),
  ];
  return fields.flatMap((entry) =>
    entry.error != null ? [{ ...entry, error: entry.error }] : [],
  );
}

/** 与煤池值不同的有效改动. */
function propEdits(row: QuoteRow): Partial<Record<IndicatorKey, number>> {
  const changed: Partial<Record<IndicatorKey, number>> = {};
  for (const key of INDICATOR_ORDER) {
    const { value } = propField(row.props[key]);
    if (value != null && value !== row.coal.props[key]) {
      changed[key] = value;
    }
  }
  return changed;
}

/** 报价相对今日估价的偏离比例; 没估价或没报价时为 0. */
function quoteDeviation(fob: number | null, today: CoalValue | undefined): number {
  return finitePositive(today?.fair) && finitePositive(fob) ? (fob - today.fair) / today.fair : 0;
}

const toDraftRow = (row: QuoteRow): QuoteDraftRow => ({
  name: row.coal.name,
  fob: row.fob,
  frt: row.frt,
  props: row.props,
});

/** 精算结果附带的提醒, 在点「精算」那一刻按当时的输入算好. */
interface QuoteNotes {
  missingFreight: number;
  lowQuotes: { name: string; percent: number }[];
}

function placeholder(quote: LatestQuote | undefined, field: "fob" | "frt"): string {
  if (quote) return `上次 ${quote[field]} (${quote.quoted_on.slice(5)})`;
  return field === "frt" ? "不填记 0" : "出厂价";
}

export function QuoteScreen({
  todayResult,
  onBack,
}: {
  todayResult: BlendResult;
  onBack: () => void;
}) {
  const [draft] = useState(() => loadQuoteDraft());
  const [batchId] = useState(() => draft?.batchId ?? crypto.randomUUID());
  const [data, setData] = useState<{ master: CoalMaster; pool: ResolvedCoal[] } | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [rows, setRows] = useState<QuoteRow[]>([]);
  const [latest, setLatest] = useState(new Map<string, LatestQuote>());
  const [snapshot, setSnapshot] = useState<(SolveSnapshot & { notes: QuoteNotes }) | null>(null);
  const [solving, setSolving] = useState(false);
  const [solveError, setSolveError] = useState<string | null>(null);
  const [archiveError, setArchiveError] = useState<string | null>(null);
  const tracker = useRef(new LatestRequestTracker());
  const archiveQueue = useRef<Promise<unknown>>(Promise.resolve());
  const resultRef = useRef<HTMLElement>(null);
  // 输到一半的中间态 (末尾逗号/小数点) 输入中不亮错; 失焦后照实报.
  const [blurred, setBlurred] = useState<ReadonlySet<string>>(new Set());
  const markBlurred = (field: string) =>
    setBlurred((prev) => (prev.has(field) ? prev : new Set(prev).add(field)));
  const showError = (entry: RowFieldError): string | null =>
    blurred.has(entry.field) || !isPartialNumberInput(entry.raw) ? entry.error : null;

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
        if (draft?.rows) {
          // 草稿里的煤若已不在可用煤池 (期间停用了), 就不再列出.
          setRows(
            draft.rows.flatMap((saved) => {
              const coal = pool.find((coal) => coal.name === saved.name);
              return coal
                ? [{ ...emptyRow(coal), fob: saved.fob, frt: saved.frt, props: saved.props }]
                : [];
            }),
          );
          return;
        }
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
        if (active) setLoadError(errorMessage(error));
      });
    void fetchLatestQuotes().then((quotes) => {
      if (active) setLatest(quotes);
    });
    return () => {
      active = false;
      tracker.current.invalidate();
    };
  }, []);

  // 煤池还没加载完就切走, 回来也要回到精算页.
  useEffect(() => {
    if (!draft) saveQuoteDraft({ todayResult, batchId, rows: null });
  }, []);

  /** 改了输入就记草稿; 由用户操作调用, 精算后清掉的草稿不会被渲染悄悄写回. */
  function rememberDraft(next: QuoteRow[]) {
    saveQuoteDraft({ todayResult, batchId, rows: next.map(toDraftRow) });
  }

  // 算完滚到结果: 手机上结果在操作条下方屏幕外, 不滚过去看不出算完了.
  useEffect(() => {
    if (snapshot) resultRef.current?.scrollIntoView?.({ behavior: "smooth", block: "start" });
  }, [snapshot]);

  /** 改了输入, 旧结果就不再对应当前报价: 作废在途求解并清掉结果. */
  function changeRows(update: (previous: QuoteRow[]) => QuoteRow[]) {
    tracker.current.invalidate();
    const next = update(rows);
    rememberDraft(next);
    setRows(next);
    setSnapshot(null);
    setSolving(false);
    setSolveError(null);
    setArchiveError(null);
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

  const pricedRows = rows.filter((row) => fobField(row.fob).value != null);
  // 能不能精算按完整校验算; 亮出来的错按"失焦/非中间态"过滤, 输到一半不闪红.
  const invalid = rows.flatMap(rowFieldErrors);
  const visibleInvalid = invalid.filter((entry) => showError(entry) != null);
  const canSolve = data?.master.price_factor && pricedRows.length >= 2 && invalid.length === 0;

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
      fob: fobField(row.fob).value ?? 0,
      frt: frtField(row.frt).value ?? 0,
      props: propEdits(row),
      purchaseTerms: termsByName.get(row.coal.name),
    }));
    const notes: QuoteNotes = {
      missingFreight: pricedRows.filter((row) => frtField(row.frt).value == null).length,
      lowQuotes: quoted.flatMap((item) => {
        const today = todayResult.coal_values?.find((value) => value.name === item.coal.name);
        const deviation = quoteDeviation(item.fob, today);
        return deviation < -QUOTE_DEVIATION_WARN
          ? [{ name: item.coal.name, percent: Math.round(-deviation * 100) }]
          : [];
      }),
    };
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
    // 同批存档按点击顺序发送, 防止慢的旧请求最后覆盖新报价; 不等待求解, 也不阻塞结果.
    const archive = archiveQueue.current
      .catch(() => undefined)
      .then(() => archiveQuotes(batchId, items));
    archiveQueue.current = archive;
    void archive.catch((error: unknown) => {
      tracker.current.accept(requestId, () => setArchiveError(errorMessage(error)));
    });
    try {
      const [{ priceFactor, benchmarksStale }, backend] = await Promise.all([
        loadPriceFactorWithStatus(data.master.price_factor),
        getBackend(),
      ]);
      if (!tracker.current.isCurrent(requestId)) return;
      const request = quoteRequest({ specs, totalQuantity, shovelTons, priceFactor }, quoted);
      const result = JSON.parse(await backend.solveJson(JSON.stringify(request))) as BlendResult;
      tracker.current.accept(requestId, () => {
        // 精算成功这次询价才算完成, 清掉草稿; 失败 (断网/超时) 留着, 切走再进还能接着填.
        clearQuoteDraft();
        setSnapshot({
          requestId,
          request,
          result,
          contractName,
          enabledCount: quoted.length,
          orphanedGuarantees: extractOrphanedGuarantees(terms),
          benchmarksStale,
          notes,
        });
      });
    } catch (error) {
      tracker.current.accept(requestId, () => setSolveError(errorMessage(error)));
    } finally {
      tracker.current.accept(requestId, () => setSolving(false));
    }
  }

  const inRecipe = new Set(todayResult.orders.map((order) => order.coal));
  return (
    <div className="quote-screen">
      <button
        className="quote-back"
        onClick={() => {
          clearQuoteDraft();
          onBack();
        }}
      >
        ← 返回今日
      </button>
      <div className="page-header">
        <div>
          <h1 className="page-title">询价精算</h1>
          <div className="page-subtitle">填上问到的出厂价, 只用这些煤算配方</div>
        </div>
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
                showError={showError}
                onEdit={(patch) => edit(row.coal.name, patch)}
                onBlurField={markBlurred}
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
            <option value="">＋ 添加煤</option>
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
        </>
      )}
      {solveError && <p role="alert">精算失败: {solveError}</p>}
      {archiveError && (
        <p role="status" className="quote-warning">
          询价未存档: {archiveError}
        </p>
      )}
      {snapshot && (
        <section className="quote-result-section" ref={resultRef} aria-label="精算结果">
          <h2 className="quote-section-title">精算结果</h2>
          {snapshot.notes.missingFreight > 0 && (
            <p className="result-notice">{snapshot.notes.missingFreight} 种煤未填运费, 按 0 计</p>
          )}
          {snapshot.notes.lowQuotes.map((quote) => (
            <p key={quote.name} className="result-notice" role="alert">
              {quote.name} 报价比今日估价低 {quote.percent}%, 请核对是否填错 (不影响本次精算)
            </p>
          ))}
          {snapshot.result.ok && snapshot.result.cost ? (
            <>
              <div className="today-dashboard">
                <div className="today-cost">
                  <CostCard
                    cost={snapshot.result.cost}
                    orphanedGuarantees={snapshot.orphanedGuarantees}
                    benchmarksStale={snapshot.benchmarksStale}
                  />
                </div>
                <RecipeCard
                  result={snapshot.result}
                  coalTypes={coalTypeMap(snapshot.request)}
                  title="精算配方"
                  shovelTons={snapshot.request.shovel_tons}
                />
                <IndicatorCard result={snapshot.result} />
              </div>
              <QuoteComparison snapshot={snapshot} />
            </>
          ) : (
            <>
              <InfeasiblePanel
                result={snapshot.result}
                contractIncomplete={
                  !snapshot.result.ok && isContractIncomplete(snapshot.request.specs)
                }
              />
              {snapshot.result.warnings.length > 0 && (
                <div className="binding-list quote-warning">
                  <div className="binding-list-title">提示</div>
                  {snapshot.result.warnings.map((warning, index) => (
                    <div key={index}>{friendlySolverText(warning)}</div>
                  ))}
                </div>
              )}
            </>
          )}
        </section>
      )}
      {/* 操作条放在整页最后、贴底常驻: 煤一多要翻好几屏, 看完结果回头改价也随手能再算;
          放最后它就不会随滚动跑到顶部行情条底下. */}
      {data && (
        <div className="quote-actionbar">
          <span className="quote-actionbar-text">
            {visibleInvalid.length > 0
              ? `先改正: ${visibleInvalid.map((entry) => entry.label).join("、")}`
              : pricedRows.length < 2
                ? "至少填两种煤的出厂价"
                : `已填 ${pricedRows.length} 种煤的出厂价`}
          </span>
          <button
            className="btn btn-primary"
            disabled={!canSolve || solving}
            onClick={() => void runSolve()}
          >
            {solving ? "精算中..." : "精算"}
          </button>
        </div>
      )}
    </div>
  );
}

/** 一种煤的询价: 出厂价、运费, 可展开改这批指标. */
function QuoteRowCard({
  row,
  today,
  inRecipe,
  latest,
  showError,
  onEdit,
  onBlurField,
  onRemove,
  onToggle,
}: {
  row: QuoteRow;
  /** 今日估价结果里这种煤的估价与配煤价值. */
  today: CoalValue | undefined;
  inRecipe: boolean;
  latest: LatestQuote | undefined;
  showError: (entry: RowFieldError) => string | null;
  onEdit: (patch: Partial<QuoteRow>) => void;
  onBlurField: (field: string) => void;
  onRemove: () => void;
  onToggle: () => void;
}) {
  const name = row.coal.name;
  const fob = fobField(row.fob);
  const frt = frtField(row.frt);
  const fobError =
    fob.error != null
      ? showError({ field: `${name}:fob`, label: "", raw: row.fob, error: fob.error })
      : null;
  const frtError =
    frt.error != null
      ? showError({ field: `${name}:frt`, label: "", raw: row.frt, error: frt.error })
      : null;
  const deviation = quoteDeviation(fob.value, today);
  const changed = propEdits(row);
  const freightOnly = fob.value == null && fob.error == null && row.frt.trim() !== "";
  return (
    <section className="card quote-row" aria-label={`询价 ${name}`}>
      <div className="quote-heading">
        <strong>{name}</strong>
        <span>今日估价 {finitePositive(today?.fair) ? Math.round(today.fair) : "—"} 元</span>
        <button className="quote-remove" aria-label={`去掉 ${name}`} onClick={onRemove}>
          去掉
        </button>
      </div>
      <div className="quote-prices">
        <label>
          出厂价
          <input
            type="text"
            inputMode="decimal"
            aria-label={`${name} 出厂价`}
            aria-invalid={fobError != null}
            value={row.fob}
            placeholder={placeholder(latest, "fob")}
            onChange={(e) => onEdit({ fob: e.target.value })}
            onBlur={() => onBlurField(`${name}:fob`)}
          />
        </label>
        <label>
          运费
          <input
            type="text"
            inputMode="decimal"
            aria-label={`${name} 运费`}
            aria-invalid={frtError != null}
            value={row.frt}
            placeholder={placeholder(latest, "frt")}
            onChange={(e) => onEdit({ frt: e.target.value })}
            onBlur={() => onBlurField(`${name}:frt`)}
          />
        </label>
      </div>
      {fobError && <p className="quote-warning">{fobError}</p>}
      {frtError && <p className="quote-warning">{frtError}</p>}
      {freightOnly && <p className="quote-hint">没填出厂价, 这种煤不参与</p>}
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
        className="quote-toggle"
        aria-label={`改这批指标 ${name}`}
        aria-expanded={row.expanded}
        onClick={onToggle}
      >
        {row.expanded ? "收起指标 ▴" : "改这批指标 ▾"}
      </button>
      {row.expanded && (
        <div className="quote-props">
          {INDICATOR_ORDER.map((key) => {
            const text = row.props[key] ?? String(row.coal.props[key] ?? "");
            const { error } = propField(row.props[key]);
            const visible =
              error != null
                ? showError({ field: `${name}:prop:${key}`, label: "", raw: text, error })
                : null;
            return (
              <label key={key}>
                {INDICATOR_LABEL[key]}{" "}
                {changed[key] != null && <span className="quote-changed">{key} 已改</span>}
                <input
                  type="text"
                  inputMode="decimal"
                  aria-label={`${name} ${key}`}
                  aria-invalid={visible != null}
                  value={text}
                  onChange={(e) => onEdit({ props: { ...row.props, [key]: e.target.value } })}
                  onBlur={() => onBlurField(`${name}:prop:${key}`)}
                />
                {visible && <span className="quote-warning">{visible}</span>}
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
  // 先取整再判高低: 差 0.3 元显示"持平", 不能是"高 0 元".
  const rounded = Math.round(gap);
  if (rounded === 0) return "持平";
  return `${rounded > 0 ? "高" : "低"} ${Math.abs(rounded)} 元`;
}
