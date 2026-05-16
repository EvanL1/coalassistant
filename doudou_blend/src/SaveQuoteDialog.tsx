/**
 * 把当前求解结果保存为报价单.
 * 入口: TodayScreen 求解成功后, 点"保存为报价".
 *
 * 流程:
 *   1. 选客户 (从客户库, 支持搜索; 没客户提示先去客户屏加)
 *   2. 填利润加成 (元/吨, 默认 0)
 *   3. 总吨数预填 (来自 TodayScreen 的 total_quantity, 可改)
 *   4. 保存 → 报价单状态 = draft
 */
import { useEffect, useMemo, useState } from "react";
import type { Customer, Quote } from "./types";
import { getCustomers, refreshCustomers, upsertQuote } from "./storage";

interface Props {
  recipe: Record<string, number>;
  cost_cif: number;
  total_tons?: number | null;
  contract_name?: string | null;
  onClose: () => void;
  onSaved?: (quote: Quote) => void;
}

function newId(): string {
  return (
    (globalThis.crypto?.randomUUID && globalThis.crypto.randomUUID()) ||
    `q_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
  );
}

function normalize(s: string): string {
  return s.replace(/　/g, " ").trim().toLowerCase();
}

export function SaveQuoteDialog({
  recipe,
  cost_cif,
  total_tons,
  contract_name,
  onClose,
  onSaved,
}: Props) {
  const [customers, setCustomers] = useState<Customer[]>(getCustomers());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [markup, setMarkup] = useState("0");
  const [tons, setTons] = useState(
    total_tons != null ? String(total_tons) : "",
  );
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void refreshCustomers().then((list) => setCustomers(list));
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  const filteredCustomers = useMemo(() => {
    const q = normalize(search);
    if (!q) return customers;
    return customers.filter((c) =>
      normalize(`${c.name} ${c.contact ?? ""} ${c.phone ?? ""}`).includes(q),
    );
  }, [customers, search]);

  const markupNum = parseFloat(markup) || 0;
  const tonsNum = tons.trim() ? parseFloat(tons) : null;
  const quotedPrice = cost_cif + markupNum;

  const canSubmit = selectedId != null && !submitting;

  async function submit() {
    if (!canSubmit) return;
    const customer = customers.find((c) => c.id === selectedId);
    if (!customer) return;
    setSubmitting(true);
    setError(null);
    const q: Quote = {
      id: newId(),
      customer_id: customer.id,
      customer_name: customer.name,
      recipe,
      cost_cif,
      markup: markupNum,
      quoted_price: quotedPrice,
      total_tons: Number.isFinite(tonsNum) ? tonsNum : null,
      contract_name: contract_name ?? null,
      status: "draft",
      note: null,
    };
    try {
      await upsertQuote(q);
      onSaved?.(q);
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
            <div className="modal-title">保存为报价单</div>
            <div className="modal-subtitle">
              选客户 + 填加成, 后续在「报价」里能找到 / 改 / 打印
            </div>
          </div>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>

        <div className="modal-body">
          {customers.length === 0 ? (
            <div
              style={{
                padding: 24,
                textAlign: "center",
                color: "var(--c-text-3)",
                fontSize: 13,
                lineHeight: 1.6,
              }}
            >
              还没有客户.
              <br />
              先去「客户」屏加一个再来.
            </div>
          ) : (
            <>
              <div className="edit-section">
                <div className="edit-section-title">客户</div>
                <input
                  type="search"
                  className="search-input"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="搜索客户名 / 联系人 / 电话"
                  style={{ marginBottom: 8 }}
                />
                <div
                  style={{
                    maxHeight: 220,
                    overflowY: "auto",
                    border: "1px solid var(--c-border)",
                    borderRadius: 8,
                  }}
                >
                  {filteredCustomers.length === 0 ? (
                    <div
                      style={{
                        padding: 16,
                        fontSize: 12,
                        color: "var(--c-text-3)",
                        textAlign: "center",
                      }}
                    >
                      没找到匹配的客户
                    </div>
                  ) : (
                    filteredCustomers.map((c) => (
                      <div
                        key={c.id}
                        onClick={() => setSelectedId(c.id)}
                        style={{
                          padding: "10px 12px",
                          borderBottom: "1px solid var(--c-border)",
                          cursor: "pointer",
                          background:
                            selectedId === c.id
                              ? "rgba(10, 95, 255, 0.08)"
                              : "transparent",
                        }}
                      >
                        <div style={{ fontSize: 14, fontWeight: 600 }}>
                          {c.name}
                          {selectedId === c.id && (
                            <span style={{ marginLeft: 6, color: "var(--c-primary)" }}>✓</span>
                          )}
                        </div>
                        {(c.contact || c.phone) && (
                          <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
                            {c.contact}
                            {c.contact && c.phone && " · "}
                            {c.phone}
                          </div>
                        )}
                      </div>
                    ))
                  )}
                </div>
              </div>

              <div className="edit-section">
                <div className="edit-section-title">价格</div>
                <div className="edit-row">
                  <div className="edit-row-label">成本 CIF</div>
                  <div style={{ fontWeight: 600, color: "var(--c-text-2)" }}>
                    ¥{cost_cif.toFixed(2)} /吨
                  </div>
                </div>
                <div className="edit-row">
                  <div className="edit-row-label">利润加成</div>
                  <input
                    type="number"
                    inputMode="decimal"
                    className="edit-input"
                    value={markup}
                    onChange={(e) => setMarkup(e.target.value)}
                    placeholder="元/吨"
                  />
                </div>
                <div className="edit-row" style={{ background: "var(--c-bg)" }}>
                  <div className="edit-row-label">报价</div>
                  <div style={{ fontWeight: 700, color: "var(--c-primary)" }}>
                    ¥{quotedPrice.toFixed(2)} /吨
                  </div>
                </div>
                <div className="edit-row">
                  <div className="edit-row-label">总吨数</div>
                  <input
                    type="number"
                    inputMode="decimal"
                    className="edit-input"
                    value={tons}
                    onChange={(e) => setTons(e.target.value)}
                    placeholder="吨"
                  />
                </div>
                {tonsNum != null && Number.isFinite(tonsNum) && (
                  <div className="edit-row" style={{ background: "var(--c-bg)" }}>
                    <div className="edit-row-label">总额</div>
                    <div style={{ fontWeight: 700, color: "var(--c-primary)" }}>
                      ¥{(quotedPrice * tonsNum).toLocaleString("zh-CN", { maximumFractionDigits: 0 })}
                    </div>
                  </div>
                )}
              </div>
            </>
          )}

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
            {submitting ? "保存中..." : "保存为草稿"}
          </button>
        </div>
      </div>
    </div>
  );
}
