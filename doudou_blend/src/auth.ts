/**
 * 认证由 Rust 服务端通过 HttpOnly Cookie 管理, 密码不进前端产物.
 */

function notifyAuthChanged(): void {
  window.dispatchEvent(new CustomEvent("doudou:auth_changed"));
}

export async function isLoggedIn(): Promise<boolean> {
  try {
    const response = await fetch("/api/auth/session", {
      credentials: "same-origin",
      cache: "no-store",
    });
    if (!response.ok) return false;
    const result = (await response.json()) as { authenticated?: unknown };
    return result.authenticated === true;
  } catch {
    return false;
  }
}

export async function tryLogin(
  username: string,
  password: string,
): Promise<boolean> {
  const response = await fetch("/api/auth/login", {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username: username.trim(), password }),
  });
  if (response.status === 401) return false;
  if (!response.ok) throw new Error(`登录服务返回 HTTP ${response.status}`);
  notifyAuthChanged();
  return true;
}

export async function logout(): Promise<void> {
  try {
    await fetch("/api/auth/logout", {
      method: "POST",
      credentials: "same-origin",
    });
  } finally {
    notifyAuthChanged();
  }
}
