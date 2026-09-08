/**
 * 期货行情窄条数据层 (大商所焦煤 / 焦炭主力连续) — 纯前端.
 *
 * 数据源: 东方财富, 响应回显 Origin 带 CORS 头, 浏览器 / Tauri webview 均可直接 fetch.
 *   - 实时接口 push2  : 最新价 + 涨跌额/涨跌幅
 *   - 日K接口 push2his: 近 30 日收盘, 只用来画 sparkline
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
  /** 近 30 日收盘, 画 sparkline; 拿不到就空数组 */
  closes: number[];
  /** 最后一根日K日期 "MM-DD"; 拿不到为 null */
  lastDate: string | null;
  /** true = 本次抓取失败, 展示的是缓存旧值 */
  stale: boolean;
}

const KEY_QUOTES = "doudou_blend.index_quotes.v2";
const TTL_MS = 10 * 60 * 1000;
const POINTS = 30;

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
    "&end=20500101&lmt=60"
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
    const close = Number(parts[2]);
    if (parts[0] && Number.isFinite(close)) {
      points.push({ date: parts[0], close });
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
      ? parseEastmoneyKlines(klineResult.value, POINTS)
      : [];
  return {
    secid: symbol.secid,
    label: symbol.label,
    ...realtime,
    closes: points.map((point) => point.close),
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
