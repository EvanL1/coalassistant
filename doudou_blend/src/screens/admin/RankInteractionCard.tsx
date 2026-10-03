/**
 * 后台卡片 1 - 煤阶交互 k.
 *
 * k 写进服务端设置后对所有求解生效, 所以拟合值只给"采用"按钮, 绝不自动套用:
 * 样本少时拟合出来的 k 可能很离谱, 要人看过误差和 D 覆盖范围再决定.
 */
import { useEffect, useState } from "react";
import {
  getCalibration,
  getSettings,
  setRankInteractionK,
  type Calibration,
} from "../../admin";
import {
  cellStyle,
  errorMessage,
  errorText,
  formatDate,
  hintText,
  inputStyle,
  smallButton,
} from "./styles";

/** 3 位有效数字: 显示和"采用"写入同一个数, 看到什么就存什么. */
function roundK(k: number): number {
  return Number(k.toPrecision(3));
}

function isEnabled(k: number | null): k is number {
  return k != null && k > 0;
}

export function RankInteractionCard() {
  const [current, setCurrent] = useState<number | null | undefined>(undefined);
  const [input, setInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [calibration, setCalibration] = useState<Calibration | null>(null);
  const [calibrationError, setCalibrationError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getSettings()
      .then((settings) => {
        if (!alive) return;
        setCurrent(settings.rank_interaction_k);
        setInput(settings.rank_interaction_k?.toString() ?? "");
      })
      .catch((e: unknown) => alive && setError(errorMessage(e)));
    getCalibration()
      .then((result) => alive && setCalibration(result))
      .catch((e: unknown) => alive && setCalibrationError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, []);

  async function save(k: number | null) {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      const settings = await setRankInteractionK(k);
      setCurrent(settings.rank_interaction_k);
      setInput(settings.rank_interaction_k?.toString() ?? "");
    } catch (e: unknown) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  function handleSave() {
    const k = Number(input.trim());
    if (input.trim() === "" || !Number.isFinite(k) || k < 0) {
      setError("k 必须是 ≥ 0 的数字");
      return;
    }
    void save(k);
  }

  const currentText =
    current === undefined ? "读取中..." : isEnabled(current) ? `k = ${current}` : "未启用";

  return (
    <div className="card">
      <div className="card-title">配煤参数: 煤阶交互 k</div>
      <p style={hintText}>
        两种煤煤阶差得越远、比例越接近对半, CSR 扣得越多; 开启后对所有求解生效
      </p>
      <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 10 }}>
        当前: {currentText}
      </div>

      <div style={{ display: "flex", gap: 8 }}>
        <input
          type="number"
          inputMode="decimal"
          min={0}
          step="any"
          aria-label="煤阶交互 k"
          value={input}
          placeholder="k ≥ 0"
          onChange={(e) => setInput(e.target.value)}
          style={{ ...inputStyle, flex: 1 }}
        />
        <button
          className="btn btn-primary"
          style={smallButton}
          disabled={saving}
          onClick={handleSave}
        >
          保存
        </button>
        <button
          className="btn btn-secondary"
          style={smallButton}
          disabled={saving || !isEnabled(current ?? null)}
          onClick={() => void save(null)}
        >
          关闭
        </button>
      </div>
      {error && <div style={errorText}>{error}</div>}

      <CalibrationPanel
        calibration={calibration}
        error={calibrationError}
        disabled={saving}
        onAdopt={(k) => void save(k)}
      />
    </div>
  );
}

function CalibrationPanel({
  calibration,
  error,
  disabled,
  onAdopt,
}: {
  calibration: Calibration | null;
  error: string | null;
  disabled: boolean;
  onAdopt: (k: number) => void;
}) {
  const box = {
    marginTop: 14,
    paddingTop: 12,
    borderTop: "1px solid var(--c-border, rgba(0,0,0,.08))",
  };
  if (error) {
    return (
      <div style={box}>
        <div style={errorText}>校准数据读取失败: {error}</div>
      </div>
    );
  }
  if (!calibration) {
    return (
      <div style={{ ...box, fontSize: 12, color: "var(--c-text-3)" }}>校准数据加载中...</div>
    );
  }

  const { n, skipped, k, k_std_error, d_min, d_max, recommended, reason, points } =
    calibration;
  const fitted = k != null ? roundK(k) : null;
  const fittedText =
    fitted == null
      ? "—"
      : k_std_error != null
        ? `${fitted} ± ${roundK(2 * k_std_error)}`
        : `${fitted}`;
  const rangeText =
    d_min != null && d_max != null ? `${roundK(d_min)} ~ ${roundK(d_max)}` : "—";

  return (
    <div style={box}>
      <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 6 }}>历史回填拟合</div>
      <div style={{ fontSize: 12, lineHeight: 1.8 }}>
        <div>
          样本 {n} 条 · 跳过 {skipped} 条
        </div>
        <div>拟合 k = {fittedText} (±2σ)</div>
        <div>煤阶方差 D 范围: {rangeText}</div>
        <div style={{ color: "var(--c-text-3)" }}>{reason}</div>
      </div>

      {recommended && fitted != null && (
        <button
          className="btn btn-secondary"
          style={{ ...smallButton, marginTop: 8 }}
          disabled={disabled}
          onClick={() => onAdopt(fitted)}
        >
          采用拟合值 k={fitted}
        </button>
      )}

      {points.length > 0 && (
        <div style={{ overflowX: "auto", marginTop: 10 }}>
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
            <thead>
              <tr>
                <th style={{ ...cellStyle, textAlign: "left", borderTop: "none" }}>日期</th>
                <th style={{ ...cellStyle, borderTop: "none" }}>基准CSR</th>
                <th style={{ ...cellStyle, borderTop: "none" }}>实测CSR</th>
                <th style={{ ...cellStyle, borderTop: "none" }}>煤阶方差 D</th>
              </tr>
            </thead>
            <tbody>
              {points.map((point) => (
                <tr key={point.id}>
                  <td style={{ ...cellStyle, textAlign: "left" }}>
                    {formatDate(point.occurred_at)}
                  </td>
                  <td style={cellStyle}>{point.base_csr.toFixed(1)}</td>
                  <td style={cellStyle}>{point.measured_csr.toFixed(1)}</td>
                  <td style={cellStyle}>{roundK(point.rank_variance)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
