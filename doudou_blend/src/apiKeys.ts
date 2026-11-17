/**
 * API 密钥的管理端数据层 (写煤库用的 data 密钥, 手机 Agent 用的 mcp 密钥).
 *
 * 密钥在服务端只存哈希, 明文仅在创建响应里出现一次 —— 所以创建后必须立刻让用户
 * 复制, 页面刷新就再也取不回来了.
 */

import { apiJson } from "./http";

/** 密钥用途: data = 写煤库 (调研录入), mcp = 手机 Agent 查询 + 求解 + 询价存档. */
export type KeyScope = "data" | "mcp";

export interface ApiKey {
  id: string;
  name: string;
  scope?: KeyScope;
  key_prefix: string;
  created_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
}

export async function listApiKeys(): Promise<ApiKey[]> {
  const result = await apiJson<{ keys?: ApiKey[] }>("/api/admin/keys", { cache: "no-store" });
  return result.keys ?? [];
}

/** 返回的 plaintext 是**唯一一次**能拿到明文的机会. */
export async function createApiKey(
  name: string,
  scope: KeyScope = "data",
): Promise<{ key: ApiKey; plaintext: string }> {
  return apiJson<{ key: ApiKey; plaintext: string }>("/api/admin/keys", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name, scope }),
  });
}

export async function revokeApiKey(id: string): Promise<void> {
  await apiJson(`/api/admin/keys/${encodeURIComponent(id)}`, { method: "DELETE" });
}
