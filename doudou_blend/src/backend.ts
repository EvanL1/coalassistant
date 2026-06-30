/**
 * 双后端适配器: Tauri 桌面/移动 用 IPC, 浏览器用 WASM 直调.
 *
 * 设计原则:
 *   - 统一异步接口 (即使 WASM 是同步的也包成 Promise, 防止前端代码因运行时不同而分叉)
 *   - 启动时探测一次, 之后缓存
 *   - WASM 模块只初始化一次, 后续调用零开销
 */

import { appendHistory, clearHistory as clearLocalHistory, getHistory, setMeasuredCsrLocal } from './storage';
import type { BlendResult, HistoryRecord, MixedIndicators } from './types';

type BackendKind = 'tauri' | 'wasm';

interface Backend {
  kind: BackendKind;
  solveJson: (input: string) => Promise<string>;
  getMasterJson: () => Promise<string>;
  getVersion: () => Promise<string>;
  /** 采集: 保存一次配煤方案 (含混合后指标, 回归 X). */
  saveHistory: (result: BlendResult, contractName: string, quantity: number | null) => Promise<void>;
  /** 列出历史方案 (跨后端统一形状, 倒序). */
  listHistory: () => Promise<HistoryRecord[]>;
  /** 回填: 给某条历史录入实测 CSR (回归 y). */
  setMeasuredCsr: (id: string, csrMeasured: number) => Promise<void>;
  /** 清空所有历史方案. */
  clearHistory: () => Promise<void>;
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

/** 探测当前运行环境. Tauri 注入 __TAURI_INTERNALS__ 全局, 浏览器没有. */
function detectTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

async function makeTauriBackend(): Promise<Backend> {
  const { invoke } = await import('@tauri-apps/api/core');
  return {
    kind: 'tauri',
    solveJson: async (input) => invoke<string>('solve_blend', { inputJson: input }),
    getMasterJson: async () => {
      // Tauri 端目前没暴露 master JSON. 退化方案: 直接从前端 fetch 静态 JSON.
      // 后续可以加 get_master_json command.
      const resp = await fetch('/coal_master.json');
      return resp.text();
    },
    getVersion: async () => invoke<string>('version'),
    saveHistory: async (result, contractName, quantity) => {
      await invoke('save_history', {
        occurredAt: new Date().toISOString(),
        contractName,
        costCif: result.cost?.cif_per_ton ?? 0,
        totalQuantity: quantity,
        resultJson: JSON.stringify(result),
      });
    },
    listHistory: async () => {
      const rows = await invoke<Array<{
        id: number;
        occurred_at: string;
        contract_name: string;
        cost_cif: number;
        result_json: string;
        csr_measured: number | null;
      }>>('list_history');
      return rows.map((r) => {
        let recipe: Record<string, number> = {};
        let mixed: MixedIndicators | null = null;
        try {
          const res = JSON.parse(r.result_json) as BlendResult;
          recipe = res.recipe ?? {};
          mixed = deriveMixed(res);
        } catch {
          // result_json 损坏 → 当旧记录处理 (无 recipe/mixed)
        }
        return {
          id: String(r.id),
          occurred_at: r.occurred_at,
          contract_name: r.contract_name,
          cost_cif: r.cost_cif,
          recipe,
          mixed,
          csr_measured: r.csr_measured ?? null,
        };
      });
    },
    setMeasuredCsr: async (id, csrMeasured) => {
      await invoke('set_measured_csr', { id: Number(id), csrMeasured });
    },
    clearHistory: async () => {
      await invoke('clear_history');
    },
  };
}

async function makeWasmBackend(): Promise<Backend> {
  // 动态 import 避免 Tauri build 时把 WASM 也打包进去
  const wasm = await import('blend-kit-wasm');
  // 注意 default export 是 __wbg_init, 显式调用初始化
  await wasm.default();
  return {
    kind: 'wasm',
    solveJson: async (input) => wasm.solveJson(input),
    getMasterJson: async () => wasm.getMasterJson(),
    getVersion: async () => wasm.getVersion(),
    saveHistory: async (result, contractName, _quantity) => {
      appendHistory({
        cost_cif: result.cost?.cif_per_ton ?? 0,
        recipe: result.recipe ?? {},
        contract_name: contractName,
        result,
      });
    },
    listHistory: async () =>
      getHistory().map((e) => {
        let mixed: MixedIndicators | null = null;
        try {
          if (e.result) mixed = deriveMixed(e.result);
        } catch {
          // result 结构损坏 → 当旧记录处理 (无 mixed, 不开放回填). 与 Tauri 路径一致.
        }
        return {
          id: e.id,
          occurred_at: e.occurred_at,
          contract_name: e.contract_name,
          cost_cif: e.cost_cif,
          recipe: e.recipe ?? {},
          mixed,
          csr_measured: e.csr_measured ?? null,
        };
      }),
    setMeasuredCsr: async (id, csrMeasured) => {
      setMeasuredCsrLocal(id, csrMeasured);
    },
    clearHistory: async () => {
      clearLocalHistory();
    },
  };
}

/**
 * 获取后端实例 (单例).
 * 第一次调用初始化, 后续直接返回缓存.
 */
export async function getBackend(): Promise<Backend> {
  if (cached) return cached;
  cached = detectTauri() ? await makeTauriBackend() : await makeWasmBackend();
  return cached;
}

/** 强制使用特定后端 (主要给测试用). */
export async function forceBackend(kind: BackendKind): Promise<Backend> {
  cached = kind === 'tauri' ? await makeTauriBackend() : await makeWasmBackend();
  return cached;
}

export type { Backend, BackendKind };
