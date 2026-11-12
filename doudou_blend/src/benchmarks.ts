/**
 * 基准报价 (分煤种的市场价, 带报价日) 的数据层. 估价模块按指标距离对最接近的几条
 * 基准加权换算每种煤的价 (不先分煤种); 报价日早于焦煤现货指数最新一期的, 服务端已按指数推算到今天.
 *
 * 读: 登录即可 (今日屏求解要用). 写: 管理员补录 / 删补录的 (master 的默认值删不了).
 */
import type { Benchmark, BenchmarkEntry, MasterPriceFactor, PriceFactor } from "./types";
import { placeFields } from "./domain/place";
import { apiFetch, readOk } from "./http";

async function request(path: string, init?: RequestInit): Promise<unknown> {
  const text = await readOk(await apiFetch(path, { credentials: "same-origin", ...init }));
  return text ? JSON.parse(text) : null;
}

/**
 * 当前生效的基准报价. 取不到 (离线、服务异常) 默认返回空表, 由调用方退回旧口径;
 * strict 时把异常抛给调用方, 好让界面说明这次用的是内置旧基准.
 */
export async function fetchBenchmarks(options?: { strict?: boolean }): Promise<BenchmarkEntry[]> {
  try {
    const result = (await request("/api/benchmarks", { cache: "no-store" })) as {
      benchmarks?: BenchmarkEntry[];
    };
    return result?.benchmarks ?? [];
  } catch (error) {
    if (options?.strict) throw error;
    return [];
  }
}

/** master 价格因子里只给人看、只给服务端用的字段 (与服务端 MASTER_ONLY_KEYS 一致). */
const MASTER_ONLY_KEYS = [
  "description",
  "uncertainty_note",
  "benchmarks_note",
  "index_series",
  "index_series_note",
  "matching_note",
  "display_series",
  "display_series_note",
] as const;

/** master 的价格因子去掉说明与服务端配置, 只留核心认识的字段. */
export function solverPriceFactor(master: MasterPriceFactor): PriceFactor {
  const hidden = new Set<string>(MASTER_ONLY_KEYS);
  return Object.fromEntries(
    Object.entries(master).filter(([key]) => !hidden.has(key)),
  ) as unknown as PriceFactor;
}

/** 求解请求里只带核心认识的字段. */
export function toRequestBenchmark(entry: BenchmarkEntry): Benchmark {
  return {
    name: entry.name,
    coal_type: entry.coal_type ?? undefined,
    form: entry.form ?? undefined,
    region: entry.region ?? undefined,
    ...placeFields(entry),
    props: { ...entry.props },
    price: entry.price,
    date: entry.date,
    source: entry.source ?? undefined,
    current_price: entry.current_price ?? undefined,
  };
}

export async function createBenchmark(benchmark: Benchmark): Promise<string> {
  const result = (await request("/api/admin/benchmarks", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(benchmark),
  })) as { id: string };
  return result.id;
}

export async function deleteBenchmark(id: string): Promise<void> {
  await request(`/api/admin/benchmarks/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
}
