/**
 * 屏 4 - 历史方案
 * 列出 localStorage 中保存的历史求解结果, 倒序展示.
 */
import { useEffect, useState } from "react";
import { getHistory, clearHistory, type HistoryEntry } from "../storage";

function formatDate(iso: string): string {
  const d = new Date(iso);
  const m = d.getMonth() + 1;
  const day = d.getDate();
  const h = d.getHours().toString().padStart(2, "0");
  const mm = d.getMinutes().toString().padStart(2, "0");
  return `${m}月${day}日 ${h}:${mm}`;
}

function recipeBrief(recipe: Record<string, number>): string {
  return Object.entries(recipe)
    .sort(([, a], [, b]) => b - a)
    .map(([name, r]) => `${name} ${(r * 100).toFixed(0)}%`)
    .join(" · ");
}

interface Props {
  onBack?: () => void;
}

export function HistoryScreen({ onBack }: Props) {
  const [list, setList] = useState<HistoryEntry[]>(getHistory());

  useEffect(() => {
    const refresh = () => setList(getHistory());
    window.addEventListener("doudou:history_changed", refresh);
    return () => window.removeEventListener("doudou:history_changed", refresh);
  }, []);

  return (
    <>
      <div className="page-header">
        <div>
          {onBack && (
            <button
              onClick={onBack}
              style={{
                fontSize: 13,
                color: "var(--c-text-3)",
                marginBottom: 4,
                padding: 0,
              }}
            >
              ‹ 返回我的
            </button>
          )}
          <h1 className="page-title">历史方案</h1>
          <div className="page-subtitle">
            {list.length === 0
              ? "尚无保存的方案"
              : `共 ${list.length} 条`}
          </div>
        </div>
        {list.length > 0 && (
          <button
            style={{
              fontSize: 12,
              color: "var(--c-danger)",
              padding: "4px 10px",
            }}
            onClick={() => {
              if (confirm("清空所有历史记录?")) clearHistory();
            }}
          >
            清空
          </button>
        )}
      </div>

      {list.length === 0 ? (
        <div className="empty">
          <p style={{ margin: 0 }}>还没保存过方案</p>
          <p style={{ fontSize: 12, marginTop: 8 }}>
            到「今日」tab 点 [保存方案] 后会出现在这里
          </p>
        </div>
      ) : (
        list.map((entry) => (
          <div key={entry.id} className="card">
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                marginBottom: 6,
              }}
            >
              <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
                {formatDate(entry.occurred_at)}
              </div>
              <div className="contract-chip">{entry.contract_name}</div>
            </div>
            <div
              style={{
                fontSize: 20,
                fontWeight: 800,
                fontVariantNumeric: "tabular-nums",
                marginBottom: 4,
              }}
            >
              ¥{entry.cost_cif.toFixed(2)}
              <span
                style={{
                  fontSize: 11,
                  color: "var(--c-text-3)",
                  fontWeight: 400,
                  marginLeft: 6,
                }}
              >
                元/吨
              </span>
            </div>
            <div style={{ fontSize: 12, color: "var(--c-text-2)" }}>
              {recipeBrief(entry.recipe)}
            </div>
          </div>
        ))
      )}
    </>
  );
}
