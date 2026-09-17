/**
 * 后端适配器: 统一走同源 Rust HTTP API.
 *
 * 设计原则:
 *   - 统一异步接口, 屏蔽传输细节
 *   - 初始化一次, 之后缓存
 *   - 使用同源 API, 避免额外的 CORS 和环境变量配置
 */

import type { BlendResult, HistoryRecord, MeasuredQuality, MixedIndicators } from './types';

interface Backend {
  solveJson: (input: string) => Promise<string>;
  getMasterJson: () => Promise<string>;
  getVersion: () => Promise<string>;
  /** 采集: 保存一次配煤方案 (含混合后指标, 回归 X). */
  saveHistory: (result: BlendResult, contractName: string, quantity: number | null) => Promise<void>;
  /** 返回历史方案数量，不加载完整 result_json. */
  countHistory: () => Promise<number>;
  /** 列出历史方案 (跨后端统一形状, 倒序). */
  listHistory: () => Promise<HistoryRecord[]>;
  /** 回填: 给某条历史录入混煤实测化验 (部分字段, 只更新提供的项). */
  setMeasuredQuality: (id: string, measured: MeasuredQuality) => Promise<void>;
  /** 清空所有历史方案. */
  clearHistory: () => Promise<void>;
}

function notifyHistoryChanged(): void {
  window.dispatchEvent(new CustomEvent('doudou:history_changed'));
}

/** 从 BlendResult 的 indicator_check 抽出回归自变量 X (混合后 6 项指标). 缺任一项 → null. */
const MIXED_KEYS: ReadonlyArray<readonly [string, keyof MixedIndicators]> = [
  ['S', 's'], ['A', 'a'], ['V', 'v'], ['G', 'g'], ['Y', 'y'], ['M', 'm'],
];

function deriveMixed(result: BlendResult): MixedIndicators | null {
  const byKey = new Map(result.indicator_check.map((ic) => [ic.indicator, ic.value]));
  const out = {} as MixedIndicators;
  for (const [indicator, field] of MIXED_KEYS) {
    const v = byKey.get(indicator);
    if (v == null) return null; // 缺任一指标 → 无完整 X, 不可回填
    out[field] = v;
  }
  return out;
}

let cached: Backend | null = null;

async function requestApi(path: string, init?: RequestInit): Promise<string> {
  const response = await fetch(`/api/${path}`, init);
  const body = await response.text();
  if (!response.ok) {
    throw new Error(body || `HTTP ${response.status}`);
  }
  return body;
}

async function makeHttpBackend(): Promise<Backend> {
  type HttpHistoryRow = {
    id: string;
    occurred_at: string;
    contract_name: string;
    cost_cif: number;
    recipe: Record<string, number>;
    result: BlendResult | null;
    csr_measured: number | null;
    s_measured: number | null;
    a_measured: number | null;
    v_measured: number | null;
    g_measured: number | null;
    y_measured: number | null;
    m_measured: number | null;
  };

  return {
    solveJson: async (input) =>
      requestApi('solve', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: input,
      }),
    getMasterJson: async () => requestApi('master'),
    getVersion: async () => requestApi('version'),
    saveHistory: async (result, contractName, quantity) => {
      await requestApi('history', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          result,
          contract_name: contractName,
          quantity,
        }),
      });
      notifyHistoryChanged();
    },
    countHistory: async () => {
      const response = JSON.parse(await requestApi('history/count')) as { count?: unknown };
      if (typeof response.count !== 'number') throw new Error('历史数量格式无效');
      return response.count;
    },
    listHistory: async () => {
      const rows = JSON.parse(await requestApi('history')) as HttpHistoryRow[];
      if (!Array.isArray(rows)) throw new Error('历史列表格式无效');
      return rows.map((row) => {
        let mixed: MixedIndicators | null = null;
        try {
          if (row.result) mixed = deriveMixed(row.result);
        } catch {
          // result 结构损坏 → 当旧记录处理 (无 mixed, 不开放回填).
        }
        return {
          id: row.id,
          occurred_at: row.occurred_at,
          contract_name: row.contract_name,
          cost_cif: row.cost_cif,
          recipe: row.recipe ?? {},
          mixed,
          csr_measured: row.csr_measured ?? null,
          s_measured: row.s_measured ?? null,
          a_measured: row.a_measured ?? null,
          v_measured: row.v_measured ?? null,
          g_measured: row.g_measured ?? null,
          y_measured: row.y_measured ?? null,
          m_measured: row.m_measured ?? null,
        };
      });
    },
    setMeasuredQuality: async (id, measured) => {
      await requestApi(`history/${encodeURIComponent(id)}/measured`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(measured),
      });
      notifyHistoryChanged();
    },
    clearHistory: async () => {
      await requestApi('history', { method: 'DELETE' });
      notifyHistoryChanged();
    },
  };
}

/**
 * 获取后端实例 (单例).
 * 第一次调用初始化, 后续直接返回缓存.
 */
export async function getBackend(): Promise<Backend> {
  if (cached) return cached;
  cached = await makeHttpBackend();
  return cached;
}

/** 丢掉缓存重新建一个后端 (主要给测试用). */
export async function forceBackend(): Promise<Backend> {
  cached = await makeHttpBackend();
  return cached;
}

export type { Backend };
