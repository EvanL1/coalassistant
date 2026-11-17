/**
 * 应用内事件: 数据层写完后通知界面与云端同步.
 *
 * 事件名只在这里出现 —— 别处手打字面量, 打错字不会报错, 只会让那个屏幕永远收不到刷新.
 * 别处用下面的短名 (类型会查), 由 scripts/check_frontend_rules.mjs 把关.
 */
import { useEffect, useRef } from "react";

/** 会写回云端的用户数据 (storage.UserStorageSnapshot 的各部分) 变了. */
export const USER_STORAGE_EVENTS = [
  "prefs_changed",
  "contract_changed",
  "quantity_changed",
  "user_coals_changed",
  "shovel_changed",
] as const;

export const APP_EVENTS = [
  ...USER_STORAGE_EVENTS,
  /** 历史方案增删或回填了实测值. */
  "history_changed",
  /** 采购扣款模板 (只存本机) 变了. */
  "penalty_template_changed",
  /** 登录或登出. */
  "auth_changed",
  /** 某个接口回了 401: 会话已失效. */
  "session_expired",
  /** 云端同步状态 (空闲 / 同步中 / 失败 / 冲突) 变了. */
  "sync_status_changed",
] as const;

export type AppEvent = (typeof APP_EVENTS)[number];

function domName(name: AppEvent): string {
  return `doudou:${name}`;
}

export function emit(name: AppEvent): void {
  window.dispatchEvent(new CustomEvent(domName(name)));
}

/** 订阅一组事件, 返回退订函数. 组件里用 useAppEvents. */
export function listen(names: readonly AppEvent[], handler: () => void): () => void {
  const listener = () => handler();
  for (const name of names) window.addEventListener(domName(name), listener);
  return () => {
    for (const name of names) window.removeEventListener(domName(name), listener);
  };
}

/** 组件挂载期间订阅一组事件; handler 每次都调最新的那个, 不必自己记依赖. */
export function useAppEvents(names: readonly AppEvent[], handler: () => void): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  const key = names.join(" ");
  useEffect(() => listen(key.split(" ") as AppEvent[], () => latest.current()), [key]);
}
