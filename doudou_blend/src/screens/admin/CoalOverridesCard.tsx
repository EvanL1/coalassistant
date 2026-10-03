/**
 * 后台卡片 3 - 煤库数据维护 (覆盖层).
 *
 * 只填空: 基线 master 已有的指标服务端会拒绝, 拒绝理由原样展示.
 * 任何改动后都要 invalidateMaster(), 否则其他屏还拿着旧的 master 缓存.
 */
import { useEffect, useState } from "react";
import {
  addOverrides,
  CONFIDENCE_LEVELS,
  deleteOverride,
  listOverrides,
  OVERRIDE_FIELDS,
  type CoalOverride,
  type OverrideConfidence,
  type OverrideField,
} from "../../admin";
import { invalidateMaster, loadMaster } from "../../master_loader";
import {
  cellStyle,
  errorMessage,
  errorText,
  formatDate,
  hintText,
  inputStyle,
  smallButton,
} from "./styles";

const CONFIDENCE_LABEL: Record<string, string> = {
  high: "高",
  medium: "中",
  low: "低",
};

export function CoalOverridesCard() {
  const [overrides, setOverrides] = useState<CoalOverride[] | null>(null);
  const [coalNames, setCoalNames] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [coal, setCoal] = useState("");
  const [field, setField] = useState<OverrideField>("CSR");
  const [value, setValue] = useState("");
  const [source, setSource] = useState("");
  const [confidence, setConfidence] = useState<OverrideConfidence>("medium");

  useEffect(() => {
    let alive = true;
    listOverrides()
      .then((rows) => alive && setOverrides(rows))
      .catch((e: unknown) => alive && setError(errorMessage(e)));
    loadMaster()
      .then((master) => {
        if (!alive) return;
        const names = master.coals.map((c) => c.name);
        setCoalNames(names);
        setCoal((previous) => previous || (names[0] ?? ""));
      })
      .catch((e: unknown) => alive && setError(`煤库读取失败: ${errorMessage(e)}`));
    return () => {
      alive = false;
    };
  }, []);

  /** 写操作统一收口: 防重入 → 执行 → 失效 master 缓存 → 重拉列表. */
  async function mutate(action: () => Promise<void>): Promise<boolean> {
    if (busy) return false;
    setBusy(true);
    setError(null);
    try {
      await action();
      invalidateMaster();
      setOverrides(await listOverrides());
      return true;
    } catch (e: unknown) {
      setError(errorMessage(e));
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function handleAdd() {
    const numeric = Number(value.trim());
    if (!coal) return setError("请选择煤种");
    if (value.trim() === "" || !Number.isFinite(numeric)) return setError("数值必须是数字");
    if (!source.trim()) return setError("来源必填 (化验单号 / 报告名 / 联系人)");
    const ok = await mutate(() =>
      addOverrides([{ coal, field, value: numeric, source: source.trim(), confidence }]),
    );
    if (ok) setValue("");
  }

  function handleDelete(row: CoalOverride) {
    if (!confirm(`删除「${row.coal_name}」的 ${row.field} 覆盖值? 将回到基线状态.`)) return;
    void mutate(() => deleteOverride(row.coal_name, row.field));
  }

  return (
    <div className="card">
      <div className="card-title">煤库数据维护</div>
      <p style={hintText}>只补基线 master 缺失的指标; 已有的值不能在这里改。</p>

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        <select
          aria-label="煤种"
          value={coal}
          onChange={(e) => setCoal(e.target.value)}
          style={inputStyle}
        >
          {coalNames.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
        <select
          aria-label="指标"
          value={field}
          onChange={(e) => setField(e.target.value as OverrideField)}
          style={inputStyle}
        >
          {OVERRIDE_FIELDS.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
        <input
          type="number"
          inputMode="decimal"
          step="any"
          aria-label="数值"
          placeholder="数值"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          style={inputStyle}
        />
        <select
          aria-label="可信度"
          value={confidence}
          onChange={(e) => setConfidence(e.target.value as OverrideConfidence)}
          style={inputStyle}
        >
          {CONFIDENCE_LEVELS.map((level) => (
            <option key={level} value={level}>
              可信度: {CONFIDENCE_LABEL[level]}
            </option>
          ))}
        </select>
        <input
          aria-label="来源"
          placeholder="来源 (必填)"
          maxLength={200}
          value={source}
          onChange={(e) => setSource(e.target.value)}
          style={{ ...inputStyle, gridColumn: "1 / -1" }}
        />
      </div>
      <button
        className="btn btn-primary"
        style={{ ...smallButton, marginTop: 8, width: "100%" }}
        disabled={busy}
        onClick={() => void handleAdd()}
      >
        添加
      </button>
      {error && <div style={errorText}>{error}</div>}

      <OverrideTable overrides={overrides} busy={busy} onDelete={handleDelete} />
    </div>
  );
}

function OverrideTable({
  overrides,
  busy,
  onDelete,
}: {
  overrides: CoalOverride[] | null;
  busy: boolean;
  onDelete: (row: CoalOverride) => void;
}) {
  if (overrides == null) {
    return <div style={{ fontSize: 12, color: "var(--c-text-3)", marginTop: 12 }}>加载中...</div>;
  }
  if (overrides.length === 0) {
    return (
      <div style={{ fontSize: 12, color: "var(--c-text-3)", marginTop: 12 }}>
        覆盖层为空, 全部用基线数据。
      </div>
    );
  }
  return (
    <div style={{ overflowX: "auto", marginTop: 12 }}>
      <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
        <thead>
          <tr>
            <th style={{ ...cellStyle, textAlign: "left", borderTop: "none" }}>煤种</th>
            <th style={{ ...cellStyle, borderTop: "none" }}>指标</th>
            <th style={{ ...cellStyle, borderTop: "none" }}>数值</th>
            <th style={{ ...cellStyle, textAlign: "left", borderTop: "none" }}>来源</th>
            <th style={{ ...cellStyle, borderTop: "none" }}>可信度</th>
            <th style={{ ...cellStyle, borderTop: "none" }}>更新</th>
            <th style={{ ...cellStyle, borderTop: "none" }} />
          </tr>
        </thead>
        <tbody>
          {overrides.map((row) => (
            <tr key={`${row.coal_name}/${row.field}`}>
              <td style={{ ...cellStyle, textAlign: "left" }}>{row.coal_name}</td>
              <td style={cellStyle}>{row.field}</td>
              <td style={cellStyle}>{row.value}</td>
              <td style={{ ...cellStyle, textAlign: "left", whiteSpace: "normal" }}>
                {row.source}
              </td>
              <td style={cellStyle}>{CONFIDENCE_LABEL[row.confidence] ?? row.confidence}</td>
              <td style={cellStyle}>
                {formatDate(row.updated_at)}
                {row.updated_by && (
                  <div style={{ fontSize: 10, color: "var(--c-text-3)" }}>{row.updated_by}</div>
                )}
              </td>
              <td style={cellStyle}>
                <button
                  className="btn btn-secondary"
                  style={{ height: 28, padding: "0 10px", fontSize: 12, color: "var(--c-danger)" }}
                  disabled={busy}
                  aria-label={`删除 ${row.coal_name} ${row.field}`}
                  onClick={() => onDelete(row)}
                >
                  删除
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
