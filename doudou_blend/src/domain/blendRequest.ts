import { fetchBenchmarks, solverPriceFactor, toRequestBenchmark } from "../benchmarks";
import { fetchLatestCoalIndex } from "../coal_index";
import type { BlendRequest, Coal, IndicatorKey, MasterPriceFactor, PriceFactor, PurchaseTerms, Spec } from "../types";
import { toEstimateCoal, type ResolvedCoal } from "./resolvedCoal";

/*
 * 今日页 (估价求解) 与询价精算共用的请求拼装. 两者合同、吨数、一铲、价格因子完全一样, 只差:
 * 煤放 coals 还是 extra_coals、带不带价、estimate_unpriced 开不开 —— 配方的差别只来自价格.
 */

/** 调用方在 await 前冻结本次输入. */
export interface BlendBase {
  specs: Spec[];
  totalQuantity: number;
  shovelTons: number | null;
  priceFactor: PriceFactor;
}

/**
 * 估价按基准报价 (分煤种、带报价日, 服务端已按现货指数推算到今天); 取不到就用 master 自带的
 * 默认基准. 两样都没有才退回现货指数点位当标准品价 (只是相对尺度). 系数表来自 master (数据不是代码).
 */
export async function loadPriceFactor(priceFactor: MasterPriceFactor): Promise<PriceFactor> {
  const fetched = (await fetchBenchmarks()).map(toRequestBenchmark);
  const benchmarks = fetched.length > 0 ? fetched : (priceFactor.benchmarks ?? []);
  const marketPrice = benchmarks.length === 0 ? await fetchLatestCoalIndex() : null;
  return {
    ...solverPriceFactor(priceFactor),
    ...(benchmarks.length > 0 ? { benchmarks } : {}),
    ...(marketPrice != null ? { market_price: marketPrice } : {}),
  };
}

function requestBase(base: BlendBase): BlendRequest {
  return {
    coals: [],
    specs: base.specs,
    total_quantity: base.totalQuantity,
    // 现场铲车配煤: 每种煤要么不用、要么至少一铲, 按整铲配.
    shovel_tons: base.shovelTons ?? undefined,
    truncate_decimal: true,
  };
}

/** 估价求解: 煤全在 extra_coals 里, estimate_unpriced 让核心按估算价把它们并入煤池. */
export function estimateRequest(base: BlendBase, coals: Coal[]): BlendRequest {
  return {
    ...requestBase(base),
    price_factor: { ...base.priceFactor, extra_coals: coals, estimate_unpriced: true },
  };
}

export interface QuotedCoal {
  coal: ResolvedCoal;
  fob: number;
  frt: number;
  props: Partial<Record<IndicatorKey, number>>;
  purchaseTerms?: PurchaseTerms | null;
}

/**
 * 询价精算: 只含填了出厂价的煤, 按当场报价进 LP; extra_coals 为空、不估价并入.
 * 价格因子照带, 核心据此给出每种煤的 报价 − 估价 (coal_values[].gap).
 */
export function quoteRequest(base: BlendBase, quoted: QuotedCoal[]): BlendRequest {
  return {
    ...requestBase(base),
    coals: quoted.flatMap((item) => {
      const coal = toEstimateCoal(item.coal, item.purchaseTerms);
      if (!coal || !Number.isFinite(item.fob) || item.fob <= 0) return [];
      return [{ ...coal, props: { ...coal.props, ...item.props }, fob: item.fob, frt: item.frt }];
    }),
    price_factor: { ...base.priceFactor, extra_coals: [], estimate_unpriced: false },
  };
}
