/**
 * 报价单详情对话框.
 * 功能:
 *   - 完整展示: 客户/配方/成本/加成/报价/总吨
 *   - 改状态: draft → sent → signed / lost
 *   - 改加成/备注 (其他字段不可改, 要改重新发个新报价)
 *   - 打印 / 保存为 PDF (浏览器原生 window.print)
 *   - 分享 (Web Share API, 移动端原生分享菜单 → 微信)
 *   - 删除
 */
import { useEffect, useMemo, useState } from "react";
import type { Quote, QuoteStatus } from "./types";
import { removeQuote, upsertQuote } from "./storage";
import { NewContractDialog } from "./NewContractDialog";

interface Props {
  quote: Quote;
  onClose: () => void;
}

const STATUS_LABEL: Record<QuoteStatus, string> = {
  draft: "草稿",
  sent: "已发",
  signed: "已签",
  lost: "已弃",
};

const STATUS_COLOR: Record<QuoteStatus, string> = {
  draft: "var(--c-text-3)",
  sent: "var(--c-primary)",
  signed: "#10b981",
  lost: "var(--c-danger)",
};

function fmt(n: number, digits = 2): string {
  return n.toLocaleString("zh-CN", {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}

function fmtDate(iso?: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function QuoteDetailDialog({ quote, onClose }: Props) {
  const [markup, setMarkup] = useState(String(quote.markup));
  const [note, setNote] = useState(quote.note ?? "");
  const [status, setStatus] = useState<QuoteStatus>(quote.status);
  const [saving, setSaving] = useState(false);
  const [showNewContract, setShowNewContract] = useState(false);

  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  const markupNum = useMemo(() => {
    const n = parseFloat(markup);
    return Number.isFinite(n) ? n : 0;
  }, [markup]);

  const newQuotedPrice = quote.cost_cif + markupNum;

  const dirty =
    markupNum !== quote.markup ||
    (note || null) !== (quote.note ?? null) ||
    status !== quote.status;

  async function save() {
    setSaving(true);
    try {
      await upsertQuote({
        ...quote,
        markup: markupNum,
        quoted_price: newQuotedPrice,
        note: note.trim() || null,
        status,
      });
      onClose();
    } catch (e) {
      alert(`保存失败: ${e instanceof Error ? e.message : String(e)}`);
      setSaving(false);
    }
  }

  async function onDelete() {
    if (!confirm("删除该报价单? 不可恢复.")) return;
    try {
      await removeQuote(quote.id);
      onClose();
    } catch (e) {
      alert(`删除失败: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  function printPdf() {
    document.body.classList.add("printing-quote");
    window.print();
    setTimeout(() => document.body.classList.remove("printing-quote"), 500);
  }

  async function share() {
    const lines = recipeLines(quote.recipe);
    const shareText = [
      `豆哥配煤 - 报价单`,
      ``,
      `客户: ${quote.customer_name}`,
      `日期: ${fmtDate(quote.updated_at ?? quote.created_at)}`,
      ``,
      `配方:`,
      ...lines.map((r) => `  ${r.name}  ${(r.ratio * 100).toFixed(1)}%`),
      ``,
      `成本 CIF: ¥${fmt(quote.cost_cif)} /吨`,
      `利润加成: ¥${fmt(markupNum)} /吨`,
      `报价: ¥${fmt(newQuotedPrice)} /吨`,
      quote.total_tons ? `总吨数: ${fmt(quote.total_tons, 0)} 吨` : "",
      quote.total_tons ? `总额: ¥${fmt(newQuotedPrice * quote.total_tons, 0)}` : "",
    ]
      .filter(Boolean)
      .join("\n");

    if (navigator.share) {
      try {
        await navigator.share({
          title: `报价单 - ${quote.customer_name}`,
          text: shareText,
        });
        return;
      } catch {
        // 用户取消或不支持, fallback 到复制
      }
    }
    try {
      await navigator.clipboard.writeText(shareText);
      alert("已复制到剪贴板, 可粘到微信发出");
    } catch {
      alert("分享失败, 请手动复制");
    }
  }

  return (
    <>
      <div className="modal-backdrop print-hide" onClick={onClose}>
        <div className="modal-sheet" onClick={(e) => e.stopPropagation()}>
          <div className="modal-handle" />
          <div className="modal-header">
            <div>
              <div className="modal-title">{quote.customer_name}</div>
              <div className="modal-subtitle">
                报价 ¥{fmt(newQuotedPrice)} /吨 ·{" "}
                <span style={{ color: STATUS_COLOR[status] }}>
                  {STATUS_LABEL[status]}
                </span>
              </div>
            </div>
            <button className="modal-close" onClick={onClose}>×</button>
          </div>

          <div className="modal-body">
            <div className="edit-section">
              <div className="edit-section-title">报价</div>
              <div className="edit-row">
                <div className="edit-row-label">成本 CIF</div>
                <div style={{ fontWeight: 600, color: "var(--c-text-2)" }}>
                  ¥{fmt(quote.cost_cif)} /吨
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
                  placeholder="0"
                />
              </div>
              <div className="edit-row" style={{ background: "var(--c-bg)" }}>
                <div className="edit-row-label">报价</div>
                <div style={{ fontWeight: 700, color: "var(--c-primary)" }}>
                  ¥{fmt(newQuotedPrice)} /吨
                </div>
              </div>
              {quote.total_tons != null && (
                <div className="edit-row">
                  <div className="edit-row-label">总吨数</div>
                  <div style={{ fontWeight: 600 }}>{fmt(quote.total_tons, 0)} 吨</div>
                </div>
              )}
              {quote.total_tons != null && (
                <div className="edit-row" style={{ background: "var(--c-bg)" }}>
                  <div className="edit-row-label">总额</div>
                  <div style={{ fontWeight: 700, color: "var(--c-primary)" }}>
                    ¥{fmt(newQuotedPrice * quote.total_tons, 0)}
                  </div>
                </div>
              )}
            </div>

            <div className="edit-section">
              <div className="edit-section-title">配方</div>
              {recipeLines(quote.recipe).map((r) => (
                <div className="edit-row" key={r.name}>
                  <div className="edit-row-label">{r.name}</div>
                  <div style={{ fontWeight: 600 }}>
                    {(r.ratio * 100).toFixed(1)}%
                  </div>
                </div>
              ))}
            </div>

            <div className="edit-section">
              <div className="edit-section-title">状态</div>
              <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                {(["draft", "sent", "signed", "lost"] as QuoteStatus[]).map(
                  (s) => (
                    <button
                      key={s}
                      onClick={() => setStatus(s)}
                      style={{
                        padding: "6px 12px",
                        borderRadius: 999,
                        fontSize: 12,
                        background:
                          status === s
                            ? STATUS_COLOR[s]
                            : "var(--c-bg)",
                        color: status === s ? "white" : "var(--c-text-2)",
                        fontWeight: status === s ? 600 : 400,
                      }}
                    >
                      {STATUS_LABEL[s]}
                    </button>
                  ),
                )}
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
                  placeholder="如: 客户已口头确认"
                />
              </div>
            </div>

            <div style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 8 }}>
              创建 {fmtDate(quote.created_at)} · 更新 {fmtDate(quote.updated_at)}
              {quote.contract_name && ` · 合同 ${quote.contract_name}`}
            </div>

            {(status === "signed" || status === "sent") && (
              <button
                className="btn btn-primary"
                style={{ width: "100%", marginTop: 12 }}
                onClick={() => setShowNewContract(true)}
              >
                ✓ 转合同
              </button>
            )}

            <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
              <button
                className="btn btn-secondary"
                style={{ flex: 1 }}
                onClick={share}
              >
                分享
              </button>
              <button
                className="btn btn-secondary"
                style={{ flex: 1 }}
                onClick={printPdf}
              >
                打印 / PDF
              </button>
              <button
                className="btn btn-secondary"
                style={{ color: "var(--c-danger)" }}
                onClick={onDelete}
              >
                删除
              </button>
            </div>
          </div>

          <div className="modal-footer">
            <button className="btn btn-secondary" onClick={onClose}>取消</button>
            <button
              className="btn btn-primary"
              onClick={save}
              disabled={!dirty || saving}
              style={{ opacity: dirty && !saving ? 1 : 0.4 }}
            >
              {saving ? "保存中..." : "保存"}
            </button>
          </div>
        </div>
      </div>

      {showNewContract && (
        <NewContractDialog
          quote={{ ...quote, markup: markupNum, quoted_price: newQuotedPrice, note: note.trim() || null, status }}
          onClose={() => setShowNewContract(false)}
          onCreated={() => {
            setShowNewContract(false);
            onClose();
          }}
        />
      )}

      {/* 打印专用: 屏幕上隐藏, 打印时整页显示 */}
      <div className="print-only">
        <h1 style={{ fontSize: 24, margin: 0 }}>豆哥配煤 · 报价单</h1>
        <hr style={{ margin: "12px 0" }} />
        <table style={{ width: "100%", fontSize: 14 }}>
          <tbody>
            <tr><td style={{ width: 100, color: "#666" }}>客户</td><td><b>{quote.customer_name}</b></td></tr>
            <tr><td style={{ color: "#666" }}>日期</td><td>{fmtDate(quote.updated_at ?? quote.created_at)}</td></tr>
            {quote.contract_name && <tr><td style={{ color: "#666" }}>合同</td><td>{quote.contract_name}</td></tr>}
          </tbody>
        </table>
        <h3 style={{ marginTop: 16 }}>配方</h3>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
          <thead>
            <tr style={{ borderBottom: "1px solid #ccc" }}>
              <th style={{ textAlign: "left", padding: 6 }}>煤种</th>
              <th style={{ textAlign: "right", padding: 6 }}>比例</th>
            </tr>
          </thead>
          <tbody>
            {recipeLines(quote.recipe).map((r) => (
              <tr key={r.name} style={{ borderBottom: "1px solid #eee" }}>
                <td style={{ padding: 6 }}>{r.name}</td>
                <td style={{ padding: 6, textAlign: "right" }}>{(r.ratio * 100).toFixed(1)}%</td>
              </tr>
            ))}
          </tbody>
        </table>
        <h3 style={{ marginTop: 16 }}>价格</h3>
        <table style={{ width: "100%", fontSize: 14 }}>
          <tbody>
            <tr><td style={{ color: "#666", padding: 4 }}>成本 CIF</td><td style={{ textAlign: "right" }}>¥{fmt(quote.cost_cif)} /吨</td></tr>
            <tr><td style={{ color: "#666", padding: 4 }}>利润加成</td><td style={{ textAlign: "right" }}>¥{fmt(markupNum)} /吨</td></tr>
            <tr style={{ borderTop: "1px solid #ccc", fontWeight: 700, fontSize: 16 }}>
              <td style={{ padding: 6 }}>报价</td>
              <td style={{ padding: 6, textAlign: "right" }}>¥{fmt(newQuotedPrice)} /吨</td>
            </tr>
            {quote.total_tons != null && (
              <tr><td style={{ color: "#666", padding: 4 }}>总吨数</td><td style={{ textAlign: "right" }}>{fmt(quote.total_tons, 0)} 吨</td></tr>
            )}
            {quote.total_tons != null && (
              <tr style={{ fontWeight: 700, fontSize: 18 }}>
                <td style={{ padding: 6 }}>总额</td>
                <td style={{ padding: 6, textAlign: "right", color: "#0a5fff" }}>
                  ¥{fmt(newQuotedPrice * quote.total_tons, 0)}
                </td>
              </tr>
            )}
          </tbody>
        </table>
        {note && (
          <>
            <h3 style={{ marginTop: 16 }}>备注</h3>
            <p style={{ fontSize: 13 }}>{note}</p>
          </>
        )}
        <p style={{ marginTop: 32, fontSize: 11, color: "#999" }}>
          此报价单由「豆哥配煤」生成 · {fmtDate(new Date().toISOString())}
        </p>
      </div>
    </>
  );
}

function recipeLines(recipe: Record<string, number>): { name: string; ratio: number }[] {
  return Object.entries(recipe)
    .map(([name, ratio]) => ({ name, ratio }))
    .sort((a, b) => b.ratio - a.ratio);
}
