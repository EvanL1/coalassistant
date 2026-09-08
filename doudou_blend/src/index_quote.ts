/**
 * 期货行情窄条数据层 (大商所焦煤 / 焦炭主力连续) — 纯前端.
 *
 * 数据源: 东方财富, 响应回显 Origin 带 CORS 头, 浏览器 / Tauri webview 均可直接 fetch.
 *   - 实时接口 push2  : 最新价 + 涨跌额/涨跌幅
 *   - 日K接口 push2his: 250 日序列, 画 sparkline + 算价格漂移参考
 *
 * 为什么涨跌幅必须走实时接口: 国内期货的涨跌幅基准是**昨结算价**, 不是昨收盘价.
 * 早先版本用相邻两根日K的收盘价相减, 算出来的数跟交易所/行情软件对不上, 极端
 * 情况下连涨跌方向都相反(实测 2026-09-08: 自算 +0.43% 红, 实际 -0.61% 绿).
 * 东财的 f169/f170 已经是昨结算口径, 直接用.
 *
 * 字段还原: f43(最新价) 和 f169(涨跌额) 是定点整数, 要除以 10^f59;
 * f170(涨跌幅) 恒定除以 100, 与 f59 无关.
 */

export interface KlinePoint {
  date: string; // "2026-09-08"
  open: number; // 开盘价 元/吨, 识别换月跳空要用
  close: number; // 收盘价 元/吨
}

/** 一个展示品种 */
export interface QuoteSymbol {
  /** 东财 secid */
  secid: string;
  /** 窄条上的短标签 */
  label: string;
}

/** 窄条展示的品种. 都是大商所(市场号 114)主力连续. */
export const QUOTE_SYMBOLS: readonly QuoteSymbol[] = [
  { secid: "114.jmm", label: "焦煤" },
  { secid: "114.jm", label: "焦炭" },
] as const;

/** 实时报价 (交易所口径) */
export interface RealtimeQuote {
  /** 最新价 元/吨 */
  price: number;
  /** 涨跌额, 相对昨结算 */
  change: number;
  /** 涨跌幅 %, 相对昨结算 */
  pct: number;
}

/** 一个品种的完整行情 */
export interface Quote extends RealtimeQuote {
  secid: string;
  label: string;
  /** 日K序列(升序), 画 sparkline + 算指数漂移; 拿不到就空数组 */
  points: KlinePoint[];
  /** 最后一根日K日期 "MM-DD"; 拿不到为 null */
  lastDate: string | null;
  /** true = 本次抓取失败, 展示的是缓存旧值 */
  stale: boolean;
}

const KEY_QUOTES = "doudou_blend.index_quotes.v3";
const TTL_MS = 10 * 60 * 1000;
/** sparkline 画多少点 */
export const SPARK_POINTS = 30;
/** 保留多少日K: 价格漂移参考要回溯到几个月前的报价日 */
const HISTORY_POINTS = 250;
/** 大商所焦煤主连, 价格漂移参考基准 */
const COKING_COAL_SECID = "114.jmm";

function realtimeUrl(secid: string): string {
  return (
    "https://push2.eastmoney.com/api/qt/stock/get" +
    `?secid=${encodeURIComponent(secid)}` +
    "&fields=f43,f58,f59,f169,f170"
  );
}

function klineUrl(secid: string): string {
  return (
    "https://push2his.eastmoney.com/api/qt/stock/kline/get" +
    `?secid=${encodeURIComponent(secid)}` +
    "&klt=101&fqt=0" +
    "&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57" +
    `&end=20500101&lmt=${HISTORY_POINTS}`
  );
}

function readNumber(source: Record<string, unknown>, key: string): number | null {
  const value = source[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/**
 * 解析东财实时响应 (纯函数, 无 IO).
 * f43/f169 按 f59 位小数还原; f170 固定百分之一. 任一必需字段缺失即返回 null.
 */
export function parseRealtimeQuote(json: unknown): RealtimeQuote | null {
  const data = (json as { data?: unknown })?.data;
  if (data == null || typeof data !== "object" || Array.isArray(data)) {
    return null;
  }
  const record = data as Record<string, unknown>;
  const rawPrice = readNumber(record, "f43");
  const rawChange = readNumber(record, "f169");
  const rawPct = readNumber(record, "f170");
  if (rawPrice == null || rawChange == null || rawPct == null) return null;

  const decimals = readNumber(record, "f59") ?? 0;
  // 停牌/无成交时东财回 0, 那不是真实价格
  if (rawPrice === 0) return null;
  const scale = 10 ** decimals;
  return {
    price: rawPrice / scale,
    change: rawChange / scale,
    pct: rawPct / 100,
  };
}

/** 解析东财日K (纯函数, 无 IO). klines 每条 "日期,开,收,高,低,量,额". */
export function parseEastmoneyKlines(json: unknown, take: number): KlinePoint[] {
  const klines = (json as { data?: { klines?: unknown } })?.data?.klines;
  if (!Array.isArray(klines)) return [];
  const points: KlinePoint[] = [];
  for (const line of klines) {
    if (typeof line !== "string") continue;
    const parts = line.split(",");
    const open = Number(parts[1]);
    const close = Number(parts[2]);
    if (parts[0] && Number.isFinite(open) && Number.isFinite(close)) {
      points.push({ date: parts[0], open, close });
    }
  }
  return points.slice(-take);
}

interface CachedQuotes {
  quotes: Quote[];
  fetchedAt: string;
}

function readCache(): CachedQuotes | null {
  try {
    const raw = localStorage.getItem(KEY_QUOTES);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as CachedQuotes;
    if (!Array.isArray(parsed?.quotes) || parsed.quotes.length === 0) return null;
    return parsed;
  } catch {
    return null;
  }
}

async function fetchJson(url: string): Promise<unknown> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.json();
}

/** 抓一个品种: 实时价必需, 日K可选(只影响 sparkline). */
async function fetchOne(symbol: QuoteSymbol): Promise<Quote | null> {
  const [realtimeResult, klineResult] = await Promise.allSettled([
    fetchJson(realtimeUrl(symbol.secid)),
    fetchJson(klineUrl(symbol.secid)),
  ]);
  if (realtimeResult.status !== "fulfilled") return null;
  const realtime = parseRealtimeQuote(realtimeResult.value);
  if (realtime == null) return null;

  const points =
    klineResult.status === "fulfilled"
      ? parseEastmoneyKlines(klineResult.value, HISTORY_POINTS)
      : [];
  return {
    secid: symbol.secid,
    label: symbol.label,
    ...realtime,
    points,
    lastDate: points.length > 0 ? points[points.length - 1].date.slice(5) : null,
    stale: false,
  };
}

/**
 * 取全部展示品种的行情.
 * 缓存新鲜(10 分钟内)直接返回; 抓取失败退回缓存并标 stale; 全无 → null.
 */
export async function fetchQuotes(): Promise<Quote[] | null> {
  const cached = readCache();
  if (cached && Date.now() - Date.parse(cached.fetchedAt) < TTL_MS) {
    return cached.quotes;
  }
  const settled = await Promise.all(QUOTE_SYMBOLS.map(fetchOne));
  const quotes = settled.filter((quote) => quote != null);
  if (quotes.length === 0) {
    return cached ? cached.quotes.map((q) => ({ ...q, stale: true })) : null;
  }
  try {
    localStorage.setItem(
      KEY_QUOTES,
      JSON.stringify({ quotes, fetchedAt: new Date().toISOString() }),
    );
  } catch {
    // 存不进缓存不影响本次展示
  }
  return quotes;
}

/**
 * 后复权: 剔除主力连续序列里的换月跳空.
 *
 * `jmm` 是"主力连续"——每天取当时主力合约的价, 换月那天直接跳到另一个合约的
 * 价位上. 远月比近月贵(contango), 于是拼接处凭空多出一截: 实测 2026-08-19
 * 昨收 1374.0 → 今开 1550.0, +12.81%, 而当天 JM2609 自己只从 1374 开到 1380,
 * JM2701 只从 1546.5 到 1550 —— 没有任何合约涨了 12.81%.
 *
 * 判据: 大商所焦煤涨跌停板 8% 量级, 单一合约上隔夜跳空超过它物理上不可能,
 * 所以"隔夜跳空 > limit ⇒ 换月拼接"是可靠的. 阈值宁松勿紧 —— 漏判会把合约
 * 价差当成涨价(高估), 误判只是少算一段真实行情(低估), 后者安全得多.
 *
 * 做法是把断点**之前**的历史整体缩放对齐, 当前值不动, 这样"最新价"始终等于
 * 市场真实报价, 只有基准被调整. 要求 points 按日期升序.
 */
export function rollAdjustedPoints(
  points: readonly KlinePoint[],
  limit = 0.08,
): KlinePoint[] {
  const adjusted = points.map((point) => ({ ...point }));
  for (let i = adjusted.length - 1; i > 0; i--) {
    const previousClose = points[i - 1].close;
    const open = points[i].open;
    if (!(previousClose > 0) || !(open > 0)) continue;
    const gap = (open - previousClose) / previousClose;
    if (Math.abs(gap) <= limit) continue;
    const factor = open / previousClose;
    for (let j = 0; j < i; j++) {
      adjusted[j].open *= factor;
      adjusted[j].close *= factor;
    }
  }
  return adjusted;
}

/**
 * 复权后的指数从 `date` 到最新的涨跌比例.
 * date 早于全部历史(定不出基准)时返回 null.
 */
export function indexRatioSince(
  points: readonly KlinePoint[],
  date: string,
): number | null {
  const adjusted = rollAdjustedPoints(points);
  let base: KlinePoint | null = null;
  let latest: KlinePoint | null = null;
  for (const point of adjusted) {
    if (latest == null || point.date > latest.date) latest = point;
    if (point.date > date) continue;
    if (base == null || point.date > base.date) base = point;
  }
  if (base == null || latest == null) return null;
  if (!(base.close > 0) || !(latest.close > 0)) return null;
  return latest.close / base.close;
}

/**
 * 焦煤期货自 `date` 起的涨跌比例, 供界面给"若随行情同步变动"的参考估算.
 *
 * 只做提示, 绝不进求解: 期货标的是低硫标准品, 而用户煤池按重量大头是中高硫
 * 山西煤, 两者价差自己会动. 拿它当成交价会错得离谱.
 */
export async function fetchFuturesRatioSince(
  date: string,
): Promise<number | null> {
  const quotes = await fetchQuotes();
  const cokingCoal = quotes?.find((quote) => quote.secid === COKING_COAL_SECID);
  if (cokingCoal == null || cokingCoal.points.length < 2) return null;
  return indexRatioSince(cokingCoal.points, date);
}
