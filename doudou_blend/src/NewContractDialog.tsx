/**
 * 报价单 → 合同转化对话框.
 * 入口: QuoteDetailDialog 状态切到 signed 后, "转合同"按钮.
 *
 * 字段 (按豆哥流程):
 *   - 合同号 (手填, 可选)
 *   - 开票地 (如 集宁)
 *   - 垫资方 (默认 "自己"; 也可填别的公司)
 *   - 首付比例 (默认 80, 单位 %)
 *   - 签约日期 (默认今天)
 *   - 备注
 *
 * 自动从报价单带:
 *   配方 / 单价 / 总吨数 / 总额 / 客户
 */
import { useEffect, useState } from "react";
import type { Contract, Quote } from "./types";
import { upsertContract, upsertQuote } from "./storage";

interface Props {
  quote: Quote;
  onClose: () => void;
  onCreated?: (c: Contract) => void;
}

function newId(): string {
  return (
    (globalThis.crypto?.randomUUID && globalThis.crypto.randomUUID()) ||
    `ct_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
  );
}

function todayISO(): string {
  return new Date().toISOString().slice(0, 10);
}

export function NewContractDialog({ quote, onClose, onCreated }: Props) {
  const [contractNo, setContractNo] = useState("");
  const [billingLocation, setBillingLocation] = useState("");
  const [prepayParty, setPrepayParty] = useState("自己");
  const [firstPayPct, setFirstPayPct] = useState("80");
  const [signedAt, setSignedAt] = useState(todayISO());
  const [note, setNote] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  const totalTons = quote.total_tons ?? 0;
  const unitPrice = quote.quoted_price;
  const totalAmount = unitPrice * totalTons;
  const pctNum = Math.max(0, Math.min(100, parseFloat(firstPayPct) || 0));
  const firstPayAmount = (totalAmount * pctNum) / 100;
  const tailPayAmount = totalAmount - firstPayAmount;

  const canSubmit = totalTons > 0 && !submitting;

  async function submit() {
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    const c: Contract = {
      id: newId(),
      quote_id: quote.id,
      customer_id: quote.customer_id,
      customer_name: quote.customer_name,
      contract_no: contractNo.trim() || null,
      billing_location: billingLocation.trim() || null,
      prepay_party: prepayParty.trim() || null,
      recipe: quote.recipe,
      unit_price: unitPrice,
      total_tons: totalTons,
      total_amount: totalAmount,
      first_pay_pct: pctNum,
      first_pay_amount: firstPayAmount,
      tail_pay_amount: tailPayAmount,
      signed_at: signedAt,
      status: "active",
      note: note.trim() || null,
    };
    try {
      await upsertContract(c);
      // 报价单状态推进到 signed (如果还不是)
      if (quote.status !== "signed") {
        await upsertQuote({ ...quote, status: "signed" });
      }
      onCreated?.(c);
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
            <div className="modal-title">转合同</div>
            <div className="modal-subtitle">
              {quote.customer_name} · {totalTons.toLocaleString("zh-CN")} 吨 · ¥
              {unitPrice.toFixed(2)} /吨
            </div>
          </div>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>

        <div className="modal-body">
          {totalTons === 0 && (
            <div
              style={{
                color: "var(--c-danger)",
                fontSize: 12,
                marginBottom: 8,
                padding: "8px 12px",
                background: "rgba(220, 38, 38, 0.08)",
                borderRadius: 8,
              }}
            >
              报价单没填总吨数, 转合同前请回报价单补上.
            </div>
          )}

          <div className="edit-section">
            <div className="edit-section-title">合同信息</div>
            <div className="edit-row">
              <div className="edit-row-label">合同号</div>
              <input
                type="text"
                className="edit-input"
                value={contractNo}
                onChange={(e) => setContractNo(e.target.value)}
                placeholder="如 HX2026-001"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">签约日期</div>
              <input
                type="date"
                className="edit-input"
                value={signedAt}
                onChange={(e) => setSignedAt(e.target.value)}
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">开票地</div>
              <input
                type="text"
                className="edit-input"
                value={billingLocation}
                onChange={(e) => setBillingLocation(e.target.value)}
                placeholder="如 集宁"
              />
            </div>
            <div className="edit-row">
              <div className="edit-row-label">垫资方</div>
              <input
                type="text"
                className="edit-input"
                value={prepayParty}
                onChange={(e) => setPrepayParty(e.target.value)}
                placeholder="自己 / 公司名"
              />
            </div>
          </div>

          <div className="edit-section">
            <div className="edit-section-title">付款</div>
            <div className="edit-row">
              <div className="edit-row-label">总额</div>
              <div style={{ fontWeight: 700, color: "var(--c-primary)" }}>
                ¥{totalAmount.toLocaleString("zh-CN", { maximumFractionDigits: 0 })}
              </div>
            </div>
            <div className="edit-row">
              <div className="edit-row-label">首付比例 %</div>
              <input
                type="number"
                inputMode="decimal"
                className="edit-input"
                value={firstPayPct}
                onChange={(e) => setFirstPayPct(e.target.value)}
              />
            </div>
            <div className="edit-row" style={{ background: "var(--c-bg)" }}>
              <div className="edit-row-label">首付金额</div>
              <div style={{ fontWeight: 600 }}>
                ¥{firstPayAmount.toLocaleString("zh-CN", { maximumFractionDigits: 0 })}
              </div>
            </div>
            <div className="edit-row" style={{ background: "var(--c-bg)" }}>
              <div className="edit-row-label">尾款 (化验后)</div>
              <div style={{ fontWeight: 600 }}>
                ¥{tailPayAmount.toLocaleString("zh-CN", { maximumFractionDigits: 0 })}
              </div>
            </div>
          </div>

          <div className="edit-section">
            <div className="edit-section-title">备注</div>
            <div className="edit-row">
              <input
                type="text"
                className="edit-input"
                style={{ width: "100%", textAlign: "left" }}
                value={note}
                onChange={(e) => setNote(e.target.value)}
                placeholder="如 撑边, 省欠长期, 免检煤"
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
              创建失败: {error}
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
            {submitting ? "创建中..." : "创建合同"}
          </button>
        </div>
      </div>
    </div>
  );
}
