/**
 * 同源接口的公共处理: 401 一律视为会话失效 (回登录页), 其他错误只取服务端 JSON 里的 reason.
 *
 * 登录接口的 401 是"账号或密码错误", 会话接口的 401 是"本来就没登录", 都不走这里.
 */
import { emit } from "./events";

export const SESSION_EXPIRED_MESSAGE = "登录已过期, 请重新登录";

export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

/**
 * 调 fetch, 遇到 401 通知界面会话已失效; 响应原样返回, 由调用方决定怎么处理.
 * 同源 Cookie 会话只在这里声明一次, 调用方不必各写一遍.
 */
export async function apiFetch(input: string, init?: RequestInit): Promise<Response> {
  const response = await fetch(input, { credentials: "same-origin", ...init });
  if (response.status === 401) emit("session_expired");
  return response;
}

/** 服务端没给原因时的通用说法. */
export function genericFailure(status: number): string {
  return `服务返回错误 (HTTP ${status})`;
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
    return genericFailure(status);
  }
  if (text === "" || text.startsWith("<")) return genericFailure(status);
  return text;
}

/** 读出响应体; 非 2xx 抛 ApiError (消息是 reason, 不是 JSON 原文). */
export async function readOk(response: Response): Promise<string> {
  const body = await response.text();
  if (!response.ok) throw new ApiError(reasonFromBody(body, response.status), response.status);
  return body;
}

/**
 * 请求同源接口并解析 JSON: 数据层模块都走这里, 不各写一份 "发请求 → 判状态 → 解析".
 * 非 2xx 抛 ApiError (带状态码, 消息是服务端给的原因); 响应体为空返回 null.
 */
export async function apiJson<T>(path: string, init?: RequestInit): Promise<T> {
  const body = await readOk(await apiFetch(path, init));
  return (body ? JSON.parse(body) : null) as T;
}

/** 同 apiJson, 但取不到 (断网、非 2xx、不是 JSON) 一律返回 null: 给"拿不到就不显示"的只读数据用. */
export async function apiJsonOrNull<T>(path: string, init?: RequestInit): Promise<T | null> {
  try {
    return await apiJson<T>(path, init);
  } catch {
    return null;
  }
}

/** 异常转成给人看的一句话: 不带 "Error:" 前缀, 不回显 JSON 原文. */
export function errorMessage(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  const text = raw.replace(/^(?:[A-Za-z]*Error:\s*)+/, "").trim();
  if (text.startsWith("{")) return reasonFromBody(text, 0).replace(" (HTTP 0)", "");
  return text || "操作失败, 请重试";
}
