/** 后台各卡片共用的行内样式, 与 ApiKeyCard 保持一致. */
import type { CSSProperties } from "react";

export const inputStyle: CSSProperties = {
  height: 36,
  padding: "0 10px",
  fontSize: 13,
  borderRadius: 8,
  border: "1px solid var(--c-border, rgba(0,0,0,.15))",
  background: "transparent",
  color: "inherit",
  minWidth: 0,
};

export const smallButton: CSSProperties = {
  height: 36,
  padding: "0 16px",
  fontSize: 13,
  flexShrink: 0,
};

export const hintText: CSSProperties = {
  fontSize: 11,
  color: "var(--c-text-3)",
  margin: "0 0 12px",
};

export const errorText: CSSProperties = {
  fontSize: 11,
  color: "var(--c-danger)",
  margin: "8px 0",
};

export const cellStyle: CSSProperties = {
  padding: "6px 4px",
  borderTop: "1px solid var(--c-border, rgba(0,0,0,.08))",
  textAlign: "right",
  whiteSpace: "nowrap",
};

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function formatDate(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleDateString("zh-CN");
}
