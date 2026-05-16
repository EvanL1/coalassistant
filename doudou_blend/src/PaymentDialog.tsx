/**
 * 收款记录新增 / 编辑.
 * 入口: ContractDetailDialog "记一笔收款" 按钮.
 */
import { useEffect, useState } from "react";
import type { Payment, PaymentKind } from "./types";
import { apiUpsertPayment } from "./api";

interface Props {
  contractId: string;
  defaultKind?: PaymentKind;
  defaultAmount?: number;
  editing?: Payment | null;
  onClose: () => void;
  onSaved?: () => void;
}

const KIND_LABEL: Record<PaymentKind, string> = {
  first: "首付",
  tail: "尾款",
  advance: "预付",
  other: "其他",
};

function newId(): string {
  return (
    (globalThis.crypto?.randomUUID && globalThis.crypto.randomUUID()) ||
    `p_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
  );
}

function todayISO(): string {
  return new Date().toISOString().slice(0, 10);
}

export function PaymentDialog({
  contractId,
  defaultKind = "first",
  defaultAmount,
  editing,
  onClose,
  onSaved,
}: Props) {
  const [kind, setKind] = useState<PaymentKind>(editing?.kind ?? defaultKind);
  const [amount, setAmount] = useState(
    editing ? String(editing.amount) : defaultAmount ? String(defaultAmount) : "",
  );
  const [paidAt, setPaidAt] = useState(editing?.paid_at ?? todayISO());
  const [payer, setPayer] = useState(editing?.payer ?? "");
  const [method, setMethod] = useState(editing?.method ?? "打款");
  const [voucherNo, setVoucherNo] = useState(editing?.voucher_no ?? "");
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

  const amountNum = parseFloat(amount) || 0;
  const canSubmit = amountNum > 0 && paidAt.trim() !== "" && !submitting;

  async function submit() {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const p: Payment = {
      id: editing?.id ?? newId(),
      contract_id: contractId,
      kind,
      amount: amountNum,
      paid_at: paidAt,
      payer: payer.trim() || null,
      method: method.trim() || null,
      voucher_no: voucherNo.trim() || null,
      note: note.trim() || null,
    };
    try {
      await apiUpsertPayment(p);
      onSaved?.();
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
            <div className="modal-title">{editing ? "编辑收款" : "记一笔收款"}</div>
            <div className="modal-subtitle">
              {KIND_LABEL[kind]} ¥{amountNum.toLocaleString("zh-CN", { maximumFractionDigits: 0 })}
            </div>
          </div>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>

        <div className="modal-body">
          <div className="edit-section">
            <div className="edit-section-title">类型</div>
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
              {(["first", "tail", "advance", "other"] as PaymentKind[]).map((k) => (
                <button
                  key={k}
                  onClick={() => setKind(k)}
                  style={{
                    padding: "6px 14px",
                    borderRadius: 999,
                    fontSize: 13,
                    background: kind === k ? "var(--c-primary)" : "var(--c-bg)",
                    color: kind === k ? "white" : "var(--c-text-2)",
                    fontWeight: kind === k ? 600 : 400,
                  }}
                >
                  {KIND_LABEL[k]}
                </button>
              ))}
            </div>
          </div>

          <div className="edit-section">
            <div className="edit-row">
              <div className="edit-row-label">
                金额 <span style={{ color: "var(--c-danger)" }}>*</span>
              </div>
              <input
                autoFocus
                type="number"
                inputMode="decimal"
                className="edit-input"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder="元"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">
                收款日 <span style={{ color: "var(--c-danger)" }}>*</span>
              </div>
              <input
                type="date"
                className="edit-input"
                value={paidAt}
                onChange={(e) => setPaidAt(e.target.value)}
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">实际打款方</div>
              <input
                type="text"
                className="edit-input"
                value={payer ?? ""}
                onChange={(e) => setPayer(e.target.value)}
                placeholder="如 客户公司名"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">方式</div>
              <input
                type="text"
                className="edit-input"
                value={method ?? ""}
                onChange={(e) => setMethod(e.target.value)}
                placeholder="打款 / 现金 / 票据"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">单号</div>
              <input
                type="text"
                className="edit-input"
                value={voucherNo ?? ""}
                onChange={(e) => setVoucherNo(e.target.value)}
                placeholder="收据号 / 票号"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">备注</div>
              <input
                type="text"
                className="edit-input"
                value={note ?? ""}
                onChange={(e) => setNote(e.target.value)}
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
