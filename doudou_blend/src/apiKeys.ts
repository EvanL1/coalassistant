/**
 * 数据更新密钥的管理端数据层.
 *
 * 密钥在服务端只存哈希, 明文仅在创建响应里出现一次 —— 所以创建后必须立刻让用户
 * 复制, 页面刷新就再也取不回来了.
 */

export interface ApiKey {
  id: string;
  name: string;
  key_prefix: string;
  created_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
}

async function request(path: string, init?: RequestInit): Promise<unknown> {
  const response = await fetch(`/api/admin/${path}`, {
    credentials: "same-origin",
    ...init,
  });
  const text = await response.text();
  if (!response.ok) {
    let reason = text;
    try {
      reason = (JSON.parse(text) as { reason?: string }).reason ?? text;
    } catch {
      // 非 JSON 错误体, 原样回显
    }
    throw new Error(reason || `HTTP ${response.status}`);
  }
  return text ? JSON.parse(text) : null;
}

export async function listApiKeys(): Promise<ApiKey[]> {
  const result = (await request("keys", { cache: "no-store" })) as {
    keys?: ApiKey[];
  };
  return result.keys ?? [];
}

/** 返回的 plaintext 是**唯一一次**能拿到明文的机会. */
export async function createApiKey(
  name: string,
): Promise<{ key: ApiKey; plaintext: string }> {
  const result = (await request("keys", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name }),
  })) as { key: ApiKey; plaintext: string };
  return result;
}

export async function revokeApiKey(id: string): Promise<void> {
  await request(`keys/${encodeURIComponent(id)}`, { method: "DELETE" });
}
