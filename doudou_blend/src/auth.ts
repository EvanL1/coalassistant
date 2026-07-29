/**
 * Web 认证由 Rust 服务端通过 HttpOnly Cookie 管理.
 * Tauri 开发模式也复用本地 HTTP 服务，避免把密码写进前端产物.
 */

function notifyAuthChanged(): void {
  window.dispatchEvent(new CustomEvent("doudou:auth_changed"));
}

function detectTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function isLoggedIn(): Promise<boolean> {
  if (detectTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<boolean>("is_authenticated");
  }

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
  if (detectTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    const authenticated = await invoke<boolean>("login", {
      username: username.trim(),
      password,
    });
    if (authenticated) notifyAuthChanged();
    return authenticated;
  }

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
    if (detectTauri()) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("logout");
    } else {
      await fetch("/api/auth/logout", {
        method: "POST",
        credentials: "same-origin",
      });
    }
  } finally {
    notifyAuthChanged();
  }
}
