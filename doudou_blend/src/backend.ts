/**
 * 双后端适配器: Tauri 桌面/移动 用 IPC, 浏览器用 WASM 直调.
 *
 * 设计原则:
 *   - 统一异步接口 (即使 WASM 是同步的也包成 Promise, 防止前端代码因运行时不同而分叉)
 *   - 启动时探测一次, 之后缓存
 *   - WASM 模块只初始化一次, 后续调用零开销
 */

type BackendKind = 'tauri' | 'wasm';

interface Backend {
  kind: BackendKind;
  solveJson: (input: string) => Promise<string>;
  getMasterJson: () => Promise<string>;
  getVersion: () => Promise<string>;
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
