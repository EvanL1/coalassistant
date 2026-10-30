/** 指数与煤价共用的弹层外壳, 保留焦点进出与 Esc 关闭. */
import { useEffect, useRef } from "react";
import type { ReactNode } from "react";

/** 时间范围 (日历天). */
export const RANGES: [label: string, days: number][] = [
  ["1月", 30],
  ["3月", 91],
  ["6月", 182],
  ["1年", 365],
];
export const DEFAULT_RANGE = "1年";

export function rangeDays(range: string): number {
  return RANGES.find(([label]) => label === range)?.[1] ?? 365;
}

export function RangeButtons({ range, onChange }: { range: string; onChange: (range: string) => void }) {
  return (
    <div className="index-chart-ranges" role="group" aria-label="时间范围">
      {RANGES.map(([label]) => (
        <button
          key={label}
          type="button"
          aria-pressed={label === range}
          className={label === range ? "active" : undefined}
          onClick={() => onChange(label)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

export function ChartDialog({ titleId, title, subtitle, onClose, children }: {
  titleId: string;
  title: string;
  subtitle: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const closeRef = useRef<HTMLButtonElement>(null);
  // 父组件每次渲染都给新的 onClose; 走 ref, 焦点处理只在开、关时各做一次.
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  // 打开时焦点进弹层, 关闭后回到点开它的那一项; Esc 关闭.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") onCloseRef.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      opener?.focus?.();
    };
  }, []);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-handle" />
        <div className="modal-header">
          <div>
            <div className="modal-title" id={titleId}>
              {title}
            </div>
            <div className="modal-subtitle">{subtitle}</div>
          </div>
          <button ref={closeRef} className="modal-close" aria-label="关闭" onClick={onClose}>
            ×
          </button>
        </div>
        <div className="modal-body index-chart-body">
          {children}
        </div>
      </div>
    </div>
  );
}
