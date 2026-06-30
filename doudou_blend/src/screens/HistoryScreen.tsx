/**
 * 屏 4 - 历史方案
 * 列出历史方案 (后端: native SQLite / web localStorage), 倒序展示.
 * 支持回填实测焦炭 CSR —— 数据闭环第一步: 把「配比 + 混合后指标」配上事后实测 CSR.
 */
import { useEffect, useState, type CSSProperties } from "react";
import { getBackend } from "../backend";
import type { HistoryRecord } from "../types";

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
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

export function HistoryScreen() {
  const [list, setList] = useState<HistoryRecord[]>([]);
  const [loadErr, setLoadErr] = useState<string | null>(null);

  async function refresh() {
    try {
      const backend = await getBackend();
      setList(await backend.listHistory());
      setLoadErr(null);
    } catch {
      setLoadErr("加载历史失败");
    }
  }

  useEffect(() => {
    void refresh();
    const onChange = () => void refresh();
    window.addEventListener("doudou:history_changed", onChange);
    return () => window.removeEventListener("doudou:history_changed", onChange);
  }, []);

  async function clearAll() {
    if (!confirm("清空所有历史记录?")) return;
    const backend = await getBackend();
    await backend.clearHistory();
    void refresh();
  }

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">历史方案</h1>
          <div
            className="page-subtitle"
            style={loadErr ? { color: "var(--c-danger)" } : undefined}
          >
            {loadErr ?? (list.length === 0 ? "尚无保存的方案" : `共 ${list.length} 条`)}
          </div>
        </div>
        {list.length > 0 && (
          <button
            style={{ fontSize: 12, color: "var(--c-danger)", padding: "4px 10px" }}
            onClick={() => void clearAll()}
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
          <HistoryCard key={entry.id} entry={entry} onSaved={refresh} />
        ))
      )}
    </>
  );
}

const linkBtn: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  fontSize: 13,
  fontWeight: 600,
  color: "var(--c-primary)",
  cursor: "pointer",
};

const miniInput: CSSProperties = {
  width: 90,
  padding: "5px 8px",
  fontSize: 14,
  border: "1px solid var(--c-border)",
  borderRadius: 8,
  background: "var(--c-bg)",
  color: "var(--c-text)",
  fontVariantNumeric: "tabular-nums",
};

const miniSave: CSSProperties = {
  padding: "5px 12px",
  fontSize: 13,
  fontWeight: 600,
  borderRadius: 8,
  background: "var(--c-primary)",
  color: "white",
  border: "none",
  cursor: "pointer",
};

/** 单条历史卡片 + 内联回填实测 CSR. */
function HistoryCard({
  entry,
  onSaved,
}: {
  entry: HistoryRecord;
  onSaved: () => void | Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [input, setInput] = useState(
    entry.csr_measured != null ? String(entry.csr_measured) : "",
  );
  const [err, setErr] = useState<string | null>(null);

  // 列表刷新后, 非编辑态同步外部最新值 (避免 useState 初值过期).
  useEffect(() => {
    if (!editing) {
      setInput(entry.csr_measured != null ? String(entry.csr_measured) : "");
    }
  }, [entry.csr_measured, editing]);

  async function save() {
    const v = Number(input);
    if (!Number.isFinite(v) || v <= 0) {
      setErr("请输入正数");
      return;
    }
    if (v > 100) {
      setErr("CSR 量程应在 0~100");
      return;
    }
    try {
      const backend = await getBackend();
      await backend.setMeasuredCsr(entry.id, v);
      setEditing(false);
      setErr(null);
      await onSaved();
    } catch {
      setErr("保存失败，请重试");
    }
  }

  return (
    <div className="card">
      <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
        <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
          {formatDate(entry.occurred_at)}
        </div>
        {entry.contract_name && <div className="contract-chip">{entry.contract_name}</div>}
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
        <span style={{ fontSize: 11, color: "var(--c-text-3)", fontWeight: 400, marginLeft: 6 }}>
          元/吨
        </span>
      </div>
      <div style={{ fontSize: 12, color: "var(--c-text-2)" }}>{recipeBrief(entry.recipe)}</div>

      {/* 回填实测 CSR: 仅对有混合指标 (回归 X) 的记录开放 */}
      {entry.mixed != null && (
        <div style={{ marginTop: 10, paddingTop: 10, borderTop: "1px solid var(--c-border)" }}>
          {editing ? (
            <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
              <input
                type="number"
                inputMode="decimal"
                value={input}
                placeholder="实测 CSR"
                autoFocus
                style={miniInput}
                onChange={(e) => {
                  setInput(e.target.value);
                  setErr(null);
                }}
              />
              <button style={miniSave} onClick={() => void save()}>
                保存
              </button>
              <button
                style={linkBtn}
                onClick={() => {
                  setEditing(false);
                  setErr(null);
                }}
              >
                取消
              </button>
              {err && <span style={{ fontSize: 12, color: "var(--c-danger)" }}>{err}</span>}
            </div>
          ) : entry.csr_measured != null ? (
            <div style={{ fontSize: 13, display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ color: "var(--c-text-2)" }}>实测 CSR</span>
              <span style={{ fontWeight: 700, fontVariantNumeric: "tabular-nums" }}>
                {entry.csr_measured.toFixed(1)}
              </span>
              <button style={linkBtn} onClick={() => setEditing(true)}>
                ✎ 改
              </button>
            </div>
          ) : (
            <button style={linkBtn} onClick={() => setEditing(true)}>
              + 录入实测CSR
            </button>
          )}
        </div>
      )}
    </div>
  );
}
