/**
 * 屏 4 - 历史方案
 * 列出历史方案 (后端: native SQLite / web localStorage), 倒序展示.
 * 支持回填混煤实测化验 (CSR + S/A/V/G/Y/M) —— 数据闭环: 把「配比 + 预测指标」
 * 配上事后化验单, 既是信任对照 (预测 vs 实测), 也是 G 修正/CSR 回归的样本.
 */
import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { getBackend } from "../backend";
import type { HistoryRecord, MeasuredQuality } from "../types";

/** 回填字段定义: [MeasuredQuality 键, 显示标签, HistoryRecord 列]. */
const MEASURED_FIELDS = [
  ["csr", "CSR", "csr_measured"],
  ["g", "粘结G", "g_measured"],
  ["y", "胶质Y", "y_measured"],
  ["s", "硫S", "s_measured"],
  ["a", "灰A", "a_measured"],
  ["v", "挥发V", "v_measured"],
  ["m", "水分M", "m_measured"],
] as const;

type MeasuredKey = (typeof MEASURED_FIELDS)[number][0];

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
  const [clearing, setClearing] = useState(false);
  const refreshRequestRef = useRef(0);
  const clearingRef = useRef(false);
  const mountedRef = useRef(true);

  async function refresh() {
    const requestId = ++refreshRequestRef.current;
    try {
      const backend = await getBackend();
      const nextList = await backend.listHistory();
      if (
        !mountedRef.current ||
        requestId !== refreshRequestRef.current
      ) {
        return;
      }
      setList(nextList);
      setLoadErr(null);
    } catch {
      if (
        mountedRef.current &&
        requestId === refreshRequestRef.current
      ) {
        setLoadErr("加载历史失败");
      }
    }
  }

  useEffect(() => {
    mountedRef.current = true;
    void refresh();
    const onChange = () => void refresh();
    window.addEventListener("doudou:history_changed", onChange);
    return () => {
      mountedRef.current = false;
      refreshRequestRef.current += 1;
      window.removeEventListener("doudou:history_changed", onChange);
    };
  }, []);

  async function clearAll() {
    if (
      clearingRef.current ||
      !confirm("清空所有历史记录?")
    ) {
      return;
    }
    clearingRef.current = true;
    refreshRequestRef.current += 1;
    setClearing(true);
    try {
      const backend = await getBackend();
      await backend.clearHistory();
      if (mountedRef.current) {
        setList([]);
        setLoadErr(null);
      }
    } catch {
      if (mountedRef.current) {
        setLoadErr("清空历史失败，请重试");
      }
    } finally {
      clearingRef.current = false;
      if (mountedRef.current) {
        setClearing(false);
      }
    }
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
            disabled={clearing}
            onClick={() => void clearAll()}
          >
            {clearing ? "清空中..." : "清空"}
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
        <div className="history-grid">
          {list.map((entry) => (
            <HistoryCard key={entry.id} entry={entry} />
          ))}
        </div>
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

/** 从记录构造编辑表单初值: 已回填的值转字符串, 未回填为空. */
function inputsFromEntry(entry: HistoryRecord): Record<MeasuredKey, string> {
  const out = {} as Record<MeasuredKey, string>;
  for (const [key, , col] of MEASURED_FIELDS) {
    const v = entry[col];
    out[key] = v != null ? String(v) : "";
  }
  return out;
}

/** 单条历史卡片 + 内联回填混煤实测化验 (7 项均可选, 至少填一项). */
function HistoryCard({ entry }: { entry: HistoryRecord }) {
  const [editing, setEditing] = useState(false);
  const [inputs, setInputs] = useState<Record<MeasuredKey, string>>(() =>
    inputsFromEntry(entry),
  );
  const [err, setErr] = useState<string | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // 列表刷新后, 非编辑态同步外部最新值 (避免 useState 初值过期).
  useEffect(() => {
    if (!editing) setInputs(inputsFromEntry(entry));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entry, editing]);

  const filled = MEASURED_FIELDS.filter(([key]) => inputs[key].trim() !== "");

  async function save() {
    if (filled.length === 0) {
      setErr("至少填一项");
      return;
    }
    const measured: MeasuredQuality = {};
    for (const [key, label] of filled) {
      const v = Number(inputs[key]);
      if (!Number.isFinite(v) || v <= 0) {
        setErr(`${label} 请输入正数`);
        return;
      }
      if (v > 100) {
        setErr(`${label} 量程应在 0~100`);
        return;
      }
      measured[key] = v;
    }
    try {
      const backend = await getBackend();
      await backend.setMeasuredQuality(entry.id, measured);
      if (!mountedRef.current) return;
      setEditing(false);
      setErr(null);
    } catch {
      if (mountedRef.current) {
        setErr("保存失败，请重试");
      }
    }
  }

  const savedValues = MEASURED_FIELDS.filter(([, , col]) => entry[col] != null);

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

      {/* 回填混煤实测化验: 仅对有混合指标 (回归 X) 的记录开放 */}
      {entry.mixed != null && (
        <div style={{ marginTop: 10, paddingTop: 10, borderTop: "1px solid var(--c-border)" }}>
          {editing ? (
            <div>
              <div
                style={{
                  display: "grid",
                  gridTemplateColumns: "repeat(auto-fill, minmax(96px, 1fr))",
                  gap: 8,
                }}
              >
                {MEASURED_FIELDS.map(([key, label]) => (
                  <label key={key} style={{ fontSize: 11, color: "var(--c-text-2)" }}>
                    {label}
                    <input
                      type="number"
                      inputMode="decimal"
                      value={inputs[key]}
                      placeholder="—"
                      style={{ ...miniInput, width: "100%", marginTop: 2 }}
                      onChange={(e) => {
                        setInputs({ ...inputs, [key]: e.target.value });
                        setErr(null);
                      }}
                    />
                  </label>
                ))}
              </div>
              <div style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 6 }}>
                留空 = 保持原值（不会清除已录数据）
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 8 }}>
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
            </div>
          ) : savedValues.length > 0 ? (
            <div style={{ fontSize: 13, display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
              <span style={{ color: "var(--c-text-2)" }}>实测</span>
              {savedValues.map(([key, label, col]) => (
                <span key={key} style={{ fontVariantNumeric: "tabular-nums" }}>
                  {label}{" "}
                  <span style={{ fontWeight: 700 }}>
                    {/* 硫等小量纲值 (<10) 保留两位小数, 0.65 显示成 0.7 会误导数据对照 */}
                    {entry[col]!.toFixed(entry[col]! < 10 ? 2 : 1)}
                  </span>
                </span>
              ))}
              <button style={linkBtn} onClick={() => setEditing(true)}>
                ✎ 改
              </button>
            </div>
          ) : (
            <button style={linkBtn} onClick={() => setEditing(true)}>
              + 录入实测化验
            </button>
          )}
        </div>
      )}
    </div>
  );
}
