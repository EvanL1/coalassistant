/**
 * 报价单列表 - Pre 合同流程 Phase 1
 * 列表 + 状态过滤 + 搜索 + 点击查看详情
 */
import { useEffect, useMemo, useState } from "react";
import type { Quote, QuoteStatus } from "../types";
import { getQuotes, refreshQuotes } from "../storage";
import { QuoteDetailDialog } from "../QuoteDetailDialog";

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
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

function normalize(s: string): string {
  return s.replace(/　/g, " ").trim().toLowerCase();
}

export function QuotesScreen() {
  const [quotes, setQuotes] = useState<Quote[]>(getQuotes());
  const [filter, setFilter] = useState<QuoteStatus | "all">("all");
  const [query, setQuery] = useState("");
  const [viewing, setViewing] = useState<Quote | null>(null);

  useEffect(() => {
    void refreshQuotes();
    const onChange = () => {
      setQuotes(getQuotes());
      // 如果当前正打开某报价, 同步它最新状态
      setViewing((v) => (v ? getQuotes().find((q) => q.id === v.id) ?? null : null));
    };
    window.addEventListener("doudou:quotes_changed", onChange);
    return () => window.removeEventListener("doudou:quotes_changed", onChange);
  }, []);

  const counts = useMemo(() => {
    const c: Record<string, number> = { all: quotes.length };
    for (const q of quotes) c[q.status] = (c[q.status] || 0) + 1;
    return c;
  }, [quotes]);

  const filtered = useMemo(() => {
    let list = quotes;
    if (filter !== "all") list = list.filter((q) => q.status === filter);
    const qs = normalize(query);
    if (qs) list = list.filter((q) => normalize(q.customer_name).includes(qs));
    return list;
  }, [quotes, filter, query]);

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">报价</h1>
          <div className="page-subtitle">
            草稿 {counts.draft || 0} · 已发 {counts.sent || 0} · 已签{" "}
            {counts.signed || 0}
          </div>
        </div>
      </div>

      <div style={{ position: "relative", marginBottom: 10 }}>
        <input
          type="search"
          className="search-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索客户名"
        />
      </div>

      <div
        style={{
          display: "flex",
          gap: 6,
          overflowX: "auto",
          marginBottom: 12,
          paddingBottom: 4,
        }}
      >
        {(["all", "draft", "sent", "signed", "lost"] as const).map((s) => (
          <button
            key={s}
            onClick={() => setFilter(s)}
            style={{
              padding: "6px 12px",
              borderRadius: 999,
              fontSize: 12,
              whiteSpace: "nowrap",
              background:
                filter === s ? "var(--c-primary)" : "var(--c-card)",
              color: filter === s ? "white" : "var(--c-text-2)",
              fontWeight: filter === s ? 600 : 400,
              boxShadow: filter === s ? "var(--shadow-sm)" : "none",
            }}
          >
            {s === "all" ? "全部" : STATUS_LABEL[s as QuoteStatus]}{" "}
            {counts[s] || 0}
          </button>
        ))}
      </div>

      {filtered.length === 0 ? (
        <div
          style={{
            textAlign: "center",
            color: "var(--c-text-3)",
            fontSize: 13,
            padding: "40px 16px",
            lineHeight: 1.6,
          }}
        >
          {quotes.length === 0 ? (
            <>
              还没有报价单.
              <br />
              去「今日」算完配方后, 点「保存为报价」.
            </>
          ) : (
            `没有匹配的报价单`
          )}
        </div>
      ) : (
        filtered.map((q) => (
          <div
            key={q.id}
            className="card"
            style={{ marginBottom: 8, cursor: "pointer" }}
            onClick={() => setViewing(q)}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 2 }}>
                  {q.customer_name}
                </div>
                <div style={{ fontSize: 12, color: "var(--c-text-3)" }}>
                  {fmtDate(q.updated_at ?? q.created_at)}
                  {q.total_tons != null && ` · ${fmt(q.total_tons, 0)} 吨`}
                </div>
              </div>
              <div style={{ textAlign: "right" }}>
                <div style={{ fontSize: 16, fontWeight: 700, color: "var(--c-primary)" }}>
                  ¥{fmt(q.quoted_price)}
                </div>
                <div
                  style={{
                    fontSize: 11,
                    fontWeight: 600,
                    color: STATUS_COLOR[q.status],
                    marginTop: 2,
                  }}
                >
                  {STATUS_LABEL[q.status]}
                </div>
              </div>
            </div>
          </div>
        ))
      )}

      {viewing && (
        <QuoteDetailDialog quote={viewing} onClose={() => setViewing(null)} />
      )}
    </>
  );
}
