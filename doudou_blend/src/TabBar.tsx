/**
 * 底部 5 tab 导航 (主入口).
 * "合同(指标)" 和 "历史方案" 不在 TabBar, 通过 "我" 屏入口进入.
 */
export type TabId =
  | "today"
  | "customers"
  | "quotes"
  | "pool"
  | "me"
  | "contract"
  | "history";

const VISIBLE_TABS: TabId[] = ["today", "customers", "quotes", "pool", "me"];

interface Props {
  active: TabId;
  onChange: (id: TabId) => void;
}

const ICONS: Record<TabId, JSX.Element> = {
  today: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M3 9.5L12 3l9 6.5V20a1 1 0 0 1-1 1h-5v-7h-6v7H4a1 1 0 0 1-1-1V9.5z" />
    </svg>
  ),
  customers: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <circle cx="9" cy="8" r="4" />
      <path d="M2 22c0-4 3-7 7-7s7 3 7 7" />
      <circle cx="17" cy="10" r="3" />
      <path d="M22 21c0-3 -2 -5 -5 -5" />
    </svg>
  ),
  quotes: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <path d="M14 2v6h6M9 13h6M9 17h6M9 9h2" />
    </svg>
  ),
  pool: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M3 9h18M9 3v18" />
    </svg>
  ),
  me: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <circle cx="12" cy="8" r="4" />
      <path d="M4 22c0-4 4-7 8-7s8 3 8 7" />
    </svg>
  ),
  contract: <></>, // 不显示在 TabBar
  history: <></>,  // 不显示在 TabBar
};

const LABEL: Record<TabId, string> = {
  today: "今日",
  customers: "客户",
  quotes: "报价",
  pool: "煤池",
  me: "我的",
  contract: "合同",
  history: "历史",
};

import type { JSX } from "react";

export function TabBar({ active, onChange }: Props) {
  return (
    <nav className="tab-bar">
      {VISIBLE_TABS.map((id) => (
        <button
          key={id}
          className={`tab ${active === id ? "active" : ""}`}
          onClick={() => onChange(id)}
        >
          {ICONS[id]}
          <span>{LABEL[id]}</span>
        </button>
      ))}
    </nav>
  );
}
