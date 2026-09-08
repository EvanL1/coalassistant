/**
 * 价格漂移推算 — 用「锚点煤」的报价历史当自建价格指数.
 *
 * 背景: master / 用户覆盖里的 fob 是手工录入的, 一录常常几十天不动. 直接拿旧价
 * 求解会低估到厂成本(实测过: 70 天不更新, 3700 吨的单子少算约 70 万). 这里用少
 * 数几个用户主力煤(锚点)的最新报价, 按比例推算其余煤的现价.
 *
 * 为什么不用外部指数: 焦煤期货主连有换月跳空(拼接处凭空多出 12%+), 免费现货
 * 指数只覆盖低硫一档, 都对不上用户实际在用的中高硫山西煤. 锚点煤和被推算的煤
 * 是同一个市场、同一套口径、同一个录入人, 不存在基差问题, 也不会静默失效.
 *
 * 规则: 推算价 = 录入价 × (锚点最新价 ÷ 锚点在该煤录价日的价), 多锚点取比例平均.
 * 运费 frt 不漂 (物流报价与煤价无关).
 */

/** 锚点煤的一次报价 */
export interface AnchorQuote {
  /** 报价日 YYYY-MM-DD */
  date: string;
  /** 出厂价 元/吨 */
  fob: number;
}

/** 锚点集合 = 自建价格指数 */
export interface PriceAnchor {
  /** 参与推算的锚点煤名 */
  coals: string[];
  /** 每个锚点煤的报价历史, 不要求有序 */
  quotes: Record<string, AnchorQuote[]>;
}

function isUsablePrice(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

function isUsableQuote(quote: unknown): quote is AnchorQuote {
  return (
    quote != null &&
    typeof quote === "object" &&
    typeof (quote as AnchorQuote).date === "string" &&
    isUsablePrice((quote as AnchorQuote).fob)
  );
}

/** 在 date 当天或之前最近一次的报价; date 早于全部报价时返回 null. */
export function anchorPriceOn(
  quotes: readonly AnchorQuote[],
  date: string,
): number | null {
  let best: AnchorQuote | null = null;
  for (const quote of quotes ?? []) {
    if (!isUsableQuote(quote) || quote.date > date) continue;
    if (best == null || quote.date > best.date) best = quote;
  }
  return best?.fob ?? null;
}

/** 最新一次报价; 无有效报价返回 null. */
function latestPrice(quotes: readonly AnchorQuote[]): number | null {
  let best: AnchorQuote | null = null;
  for (const quote of quotes ?? []) {
    if (!isUsableQuote(quote)) continue;
    if (best == null || quote.date > best.date) best = quote;
  }
  return best?.fob ?? null;
}

/**
 * 全池漂移比例 = 各锚点 (最新价 ÷ 录价日价) 的算术平均.
 * 定位不到基准的锚点直接跳过; 全部跳过时返回 null (宁可不漂, 不瞎漂).
 */
export function driftRatio(
  anchor: PriceAnchor,
  quotedAt: string,
): number | null {
  const ratios: number[] = [];
  for (const name of anchor?.coals ?? []) {
    const quotes = anchor?.quotes?.[name];
    if (!Array.isArray(quotes)) continue;
    const base = anchorPriceOn(quotes, quotedAt);
    const now = latestPrice(quotes);
    if (base == null || now == null) continue;
    ratios.push(now / base);
  }
  if (ratios.length === 0) return null;
  return ratios.reduce((sum, r) => sum + r, 0) / ratios.length;
}

/** CoalPref 里与锚点相关的部分 (结构化声明, 避免 domain 层反向依赖 storage). */
export interface AnchorSource {
  is_price_anchor?: boolean;
  fob_history?: AnchorQuote[];
}

/** 从煤偏好里挑出被标记为锚点且有报价历史的煤, 组成自建价格指数. */
export function buildPriceAnchor(
  prefs: Record<string, AnchorSource | null | undefined>,
): PriceAnchor {
  const coals: string[] = [];
  const quotes: Record<string, AnchorQuote[]> = {};
  for (const [name, pref] of Object.entries(prefs ?? {})) {
    if (pref?.is_price_anchor !== true) continue;
    const history = Array.isArray(pref.fob_history)
      ? pref.fob_history.filter(isUsableQuote)
      : [];
    if (history.length === 0) continue;
    coals.push(name);
    quotes[name] = history;
  }
  return { coals, quotes };
}

/**
 * 推算某煤当前 fob.
 * 返回 null 表示无法推算(没锚点/没录价日/录价日早于锚点全部历史), 调用方退回录入价.
 */
export function driftedFob(
  fob: number,
  quotedAt: string | null | undefined,
  anchor: PriceAnchor,
): number | null {
  if (!isUsablePrice(fob) || !quotedAt) return null;
  const ratio = driftRatio(anchor, quotedAt);
  return ratio == null ? null : fob * ratio;
}
