/**
 * 跟 Cloudflare Pages Functions 的 API client.
 *
 * 设计:
 *   - 所有调用自动带 Authorization: Bearer <token> (token 从 localStorage)
 *   - 失败统一 throw, 让 storage 层决定要不要 fallback localStorage cache
 *   - 跑在 Tauri 桌面端时也能工作 (相对 URL → 走当前 origin), 但桌面端没后端,
 *     调用会失败而 storage 层会回退到 localStorage. 桌面端要真正多设备同步需要
 *     额外配置 API base URL, 留下一波.
 */

import type { MasterCoalEntry } from "./types";

const KEY_API_TOKEN = "doudou_blend.api_token.v1";

export function getApiToken(): string | null {
  return localStorage.getItem(KEY_API_TOKEN);
}

export function setApiToken(token: string): void {
  localStorage.setItem(KEY_API_TOKEN, token);
}

export function clearApiToken(): void {
  localStorage.removeItem(KEY_API_TOKEN);
}

async function authFetch(path: string, init: RequestInit = {}): Promise<Response> {
  const token = getApiToken();
  if (!token) throw new Error("未登录");
  const headers = new Headers(init.headers);
  headers.set("Authorization", `Bearer ${token}`);
  if (init.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  return fetch(path, { ...init, headers });
}

export interface LoginResp {
  ok: boolean;
  token?: string;
  error?: string;
}

/** 远程登录: 返回 token, 调用方负责存到 localStorage (用 setApiToken). */
export async function apiLogin(user: string, pass: string): Promise<LoginResp> {
  const resp = await fetch("/api/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ user, pass }),
  });
  return (await resp.json()) as LoginResp;
}

export async function apiListCoals(): Promise<MasterCoalEntry[]> {
  const resp = await authFetch("/api/coals");
  if (!resp.ok) throw new Error(`GET /api/coals 失败: ${resp.status}`);
  const data = (await resp.json()) as { coals: MasterCoalEntry[] };
  return data.coals;
}

export async function apiAddCoal(coal: MasterCoalEntry): Promise<void> {
  const resp = await authFetch("/api/coals", {
    method: "POST",
    body: JSON.stringify(coal),
  });
  if (!resp.ok) {
    const data = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(data.error || `POST /api/coals 失败: ${resp.status}`);
  }
}

export async function apiDeleteCoal(name: string): Promise<void> {
  const resp = await authFetch(
    `/api/coals?name=${encodeURIComponent(name)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/coals 失败: ${resp.status}`);
}

export async function apiMigrateCoals(coals: MasterCoalEntry[]): Promise<number> {
  const resp = await authFetch("/api/coals/migrate", {
    method: "POST",
    body: JSON.stringify({ coals }),
  });
  if (!resp.ok) throw new Error(`POST /api/coals/migrate 失败: ${resp.status}`);
  const data = (await resp.json()) as { imported: number };
  return data.imported;
}
