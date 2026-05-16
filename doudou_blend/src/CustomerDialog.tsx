/**
 * 客户新增 / 编辑 对话框.
 * 字段: 客户名 (必填) / 联系人 / 电话 / 备注
 */
import { useEffect, useState } from "react";
import type { Customer } from "./types";
import { upsertCustomer } from "./storage";

interface Props {
  /** null = 新增模式; 否则编辑模式 */
  editing: Customer | null;
  onClose: () => void;
}

function newId(): string {
  return (
    (globalThis.crypto?.randomUUID && globalThis.crypto.randomUUID()) ||
    `c_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
  );
}

export function CustomerDialog({ editing, onClose }: Props) {
  const [name, setName] = useState(editing?.name ?? "");
  const [contact, setContact] = useState(editing?.contact ?? "");
  const [phone, setPhone] = useState(editing?.phone ?? "");
  const [note, setNote] = useState(editing?.note ?? "");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  const canSubmit = name.trim().length > 0 && !submitting;

  async function submit() {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      await upsertCustomer({
        id: editing?.id ?? newId(),
        name: name.trim(),
        contact: contact.trim() || null,
        phone: phone.trim() || null,
        note: note.trim() || null,
      });
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
            <div className="modal-title">{editing ? "编辑客户" : "新增客户"}</div>
            <div className="modal-subtitle">
              {editing ? editing.name : "录一条客户信息, 后续报价直接选"}
            </div>
          </div>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>

        <div className="modal-body">
          <div className="edit-section">
            <div className="edit-row">
              <div className="edit-row-label">
                客户名 <span style={{ color: "var(--c-danger)" }}>*</span>
              </div>
              <input
                autoFocus
                type="text"
                className="edit-input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="如: 山东某焦化"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">联系人</div>
              <input
                type="text"
                className="edit-input"
                value={contact ?? ""}
                onChange={(e) => setContact(e.target.value)}
                placeholder="如: 王经理"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">电话</div>
              <input
                type="tel"
                className="edit-input"
                value={phone ?? ""}
                onChange={(e) => setPhone(e.target.value)}
                placeholder="138xxxxxxxx"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">备注</div>
              <input
                type="text"
                className="edit-input"
                value={note ?? ""}
                onChange={(e) => setNote(e.target.value)}
                placeholder="如: 偏好低硫"
              />
            </div>
          </div>

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
              保存失败: {error}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>取消</button>
          <button
            className="btn btn-primary"
            onClick={submit}
            disabled={!canSubmit}
            style={{ opacity: canSubmit ? 1 : 0.4 }}
          >
            {submitting ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
