/**
 * 同源接口的公共处理: 401 一律视为会话失效 (回登录页), 其他错误只取服务端 JSON 里的 reason.
 *
 * 登录接口的 401 是"账号或密码错误", 会话接口的 401 是"本来就没登录", 都不走这里.
 */

export const SESSION_EXPIRED_EVENT = "doudou:session_expired";
export const SESSION_EXPIRED_MESSAGE = "登录已过期, 请重新登录";

export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

/** 调 fetch, 遇到 401 通知界面会话已失效; 响应原样返回, 由调用方决定怎么处理. */
export async function apiFetch(input: string, init?: RequestInit): Promise<Response> {
  const response = await fetch(input, init);
  if (response.status === 401) {
    window.dispatchEvent(new CustomEvent(SESSION_EXPIRED_EVENT));
  }
  return response;
}

/** 从错误响应体里取给人看的原因: JSON 取 reason / errors, HTML 错误页不回显. */
export function reasonFromBody(body: string, status: number): string {
  if (status === 401) return SESSION_EXPIRED_MESSAGE;
  const text = body.trim();
  if (text.startsWith("{")) {
    try {
      const parsed = JSON.parse(text) as { reason?: unknown; errors?: unknown };
      if (typeof parsed.reason === "string" && parsed.reason.trim()) return parsed.reason;
      if (Array.isArray(parsed.errors) && parsed.errors.length > 0) {
        return parsed.errors.map(String).join("; ");
      }
    } catch {
      // 不是合法 JSON, 按纯文本处理
    }
    return `服务返回错误 (HTTP ${status})`;
  }
  if (text === "" || text.startsWith("<")) return `服务返回错误 (HTTP ${status})`;
  return text;
}

/** 读出响应体; 非 2xx 抛 ApiError (消息是 reason, 不是 JSON 原文). */
export async function readOk(response: Response): Promise<string> {
  const body = await response.text();
  if (!response.ok) throw new ApiError(reasonFromBody(body, response.status), response.status);
  return body;
}

/** 异常转成给人看的一句话: 不带 "Error:" 前缀, 不回显 JSON 原文. */
export function errorMessage(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  const text = raw.replace(/^(?:[A-Za-z]*Error:\s*)+/, "").trim();
  if (text.startsWith("{")) return reasonFromBody(text, 0).replace(" (HTTP 0)", "");
  return text || "操作失败, 请重试";
}
