/**
 * 新增煤种对话框.
 *
 * 入口: 煤池界面右上 + 按钮
 * 字段: 煤名 (必填) / 产地 / 煤类
 * 校验: 实时查重 (master 73 种 + 用户已新增), trim + 全角空格 + 大小写无关
 * 新煤 status 默认 = "draft", 化验值留空, 后续在 CoalEditor 里补.
 */
import { useEffect, useMemo, useState } from "react";
import type { MasterCoalEntry } from "./types";
import {
  addUserCoal,
  findDuplicateCoalName,
  normalizeCoalName,
} from "./storage";

interface Props {
  /** 当前所有已存在的煤 (master + user-added) - 用来查重 */
  existing: MasterCoalEntry[];
  onClose: () => void;
}

export function NewCoalDialog({ existing, onClose }: Props) {
  const [name, setName] = useState("");
  const [region, setRegion] = useState("");
  const [coalType, setCoalType] = useState("");

  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  const dup = useMemo(
    () => findDuplicateCoalName(name, existing),
    [name, existing],
  );

  const trimmedName = normalizeCoalName(name);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canSubmit = trimmedName.length > 0 && dup == null && !submitting;

  async function submit() {
    if (!canSubmit) return;
    const entry: MasterCoalEntry = {
      name: name.trim(),
      region: region.trim() || null,
      coal_type: coalType.trim() || null,
      status: "draft",
      props: {},
      fob: null,
      frt: null,
      note: null,
    };
    setSubmitting(true);
    setError(null);
    try {
      await addUserCoal(entry);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setSubmitting(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal-sheet" onClick={(e) => e.stopPropagation()}>
        <div className="modal-handle" />

        <div className="modal-header">
          <div>
            <div className="modal-title">新增煤种</div>
            <div className="modal-subtitle">
              录煤名后, 化验值可在煤池里点开继续补
            </div>
          </div>
          <button className="modal-close" onClick={onClose}>
            ×
          </button>
        </div>

        <div className="modal-body">
          <div className="edit-section">
            <div className="edit-row">
              <div className="edit-row-label">
                煤名 <span style={{ color: "var(--c-danger)" }}>*</span>
              </div>
              <input
                autoFocus
                type="text"
                className="edit-input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="如: 老山兰"
              />
            </div>
            {dup && (
              <div
                style={{
                  color: "var(--c-danger)",
                  fontSize: 12,
                  padding: "6px 4px 0",
                }}
              >
                ⚠ 已存在该煤种: <strong>{dup}</strong>
              </div>
            )}

            <div className="edit-row">
              <div className="edit-row-label">产地</div>
              <input
                type="text"
                className="edit-input"
                value={region}
                onChange={(e) => setRegion(e.target.value)}
                placeholder="可选, 如: 山西"
              />
            </div>

            <div className="edit-row">
              <div className="edit-row-label">煤类</div>
              <input
                type="text"
                className="edit-input"
                value={coalType}
                onChange={(e) => setCoalType(e.target.value)}
                placeholder="可选, 如: 主焦煤"
              />
            </div>
          </div>

          <p
            style={{
              fontSize: 11,
              color: "var(--c-text-3)",
              margin: "4px 4px 0",
              lineHeight: 1.5,
            }}
          >
            新煤默认状态为「待核实」, 不参与配比. 在煤池里点开补全化验值后,
            可切换为启用.
          </p>

          {error && (
            <div
              style={{
                color: "var(--c-danger)",
                fontSize: 12,
                marginTop: 8,
                padding: "8px 12px",
                background: "rgba(220, 38, 38, 0.08)",
                borderRadius: 8,
              }}
            >
              新增失败: {error}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            取消
          </button>
          <button
            className="btn btn-primary"
            onClick={submit}
            disabled={!canSubmit}
            style={{ opacity: canSubmit ? 1 : 0.4 }}
          >
            {submitting ? "新增中..." : "新增"}
          </button>
        </div>
      </div>
    </div>
  );
}
