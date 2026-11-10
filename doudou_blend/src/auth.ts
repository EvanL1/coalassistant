/**
 * 认证由 Rust 服务端通过 HttpOnly Cookie 管理, 密码不进前端产物.
 */
import { reasonFromBody } from "./http";
import { resetSyncState } from "./cloudStorage";
import { claimLocalData, clearLocalAppData, getLocalOwner } from "./localAccount";

function notifyAuthChanged(): void {
  window.dispatchEvent(new CustomEvent("doudou:auth_changed"));
}

export interface Session {
  authenticated: boolean;
  /** 管理员进后台, 不进五屏应用. 旧版服务端不返回此字段, 按 false 处理. */
  admin: boolean;
}

const SIGNED_OUT: Session = { authenticated: false, admin: false };

/** 当前会话是不是管理员: 登录或查询会话时记下, 登出时据此决定清不清本机的用户数据. */
let adminSession = false;

/** 登录被限速 (HTTP 429): 服务端带 Retry-After 头 (秒) 与 reason. */
export class LoginRateLimitError extends Error {
  /** 多少秒后可再试; 服务端没带 Retry-After 头时为 null. */
  readonly retryAfterSeconds: number | null;

  constructor(message: string, retryAfterSeconds: number | null) {
    super(message);
    this.name = "LoginRateLimitError";
    this.retryAfterSeconds = retryAfterSeconds;
  }
}

export async function getSession(): Promise<Session> {
  try {
    const response = await fetch("/api/auth/session", {
      credentials: "same-origin",
      cache: "no-store",
    });
    if (!response.ok) return SIGNED_OUT;
    const result = (await response.json()) as {
      authenticated?: unknown;
      admin?: unknown;
    };
    const authenticated = result.authenticated === true;
    adminSession = authenticated && result.admin === true;
    return { authenticated, admin: adminSession };
  } catch {
    return SIGNED_OUT;
  }
}

export async function tryLogin(
  username: string,
  password: string,
): Promise<boolean> {
  const name = username.trim();
  const response = await fetch("/api/auth/login", {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username: name, password }),
  });
  if (response.status === 401) return false;
  if (response.status === 429) {
    const body = await response.text();
    const reason = reasonFromBody(body, 429);
    const retryAfter = Number(response.headers.get("Retry-After"));
    throw new LoginRateLimitError(
      reason === "服务返回错误 (HTTP 429)" ? "尝试次数过多, 请稍后再试" : reason,
      Number.isFinite(retryAfter) && retryAfter > 0 ? Math.ceil(retryAfter) : null,
    );
  }
  if (!response.ok) throw new Error(reasonFromBody(await response.text(), response.status));
  const result = (await response.json().catch(() => ({}))) as { admin?: unknown };
  // 管理员会话进后台, 不认领、也不清空本机的用户数据 —— 管理员与普通账号
  // 常在同一台设备上轮流登录, 一认领, 只存本机的数据 (扣款模板、没同步的改动) 就没了.
  adminSession = result.admin === true;
  if (!adminSession) {
    const owner = getLocalOwner();
    if (owner != null && owner !== name) {
      const confirmed = confirm(
        `本机数据目前属于「${owner}」。用「${name}」登录会清掉本机的采购扣款模板、` +
          `还没同步到云端的改动和本机历史, 清掉无法恢复。确定换账号登录?`,
      );
      if (!confirmed) {
        // 登录已在服务端成立, 用户选了不换: 把这个会话退掉, 本机数据不动.
        await fetch("/api/auth/logout", {
          method: "POST",
          credentials: "same-origin",
        }).catch(() => undefined);
        throw new Error("已取消登录, 本机数据没动");
      }
    }
    // 必须在通知界面之前: 界面一切换就开始同步, 那时本机数据得已经属于这个账号.
    claimLocalData(name);
  }
  notifyAuthChanged();
  return true;
}

export async function logout(): Promise<void> {
  // 管理员退出后台不清本机的用户数据 (同一台设备上普通账号还要用).
  const wasAdmin = adminSession;
  try {
    await fetch("/api/auth/logout", {
      method: "POST",
      credentials: "same-origin",
    });
  } finally {
    adminSession = false;
    if (!wasAdmin) clearLocalAppData();
    resetSyncState();
    notifyAuthChanged();
  }
}
