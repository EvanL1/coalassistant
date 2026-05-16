/**
 * 合同列表 - Phase 2 主屏
 * 顶部聚合: 应收 / 已收 / 待收 (跨所有 active 合同的简化版应收账款)
 * 列表: 状态过滤 + 搜索 + 点击进详情
 */
import { useEffect, useMemo, useState } from "react";
import type { Contract, ContractStatus, Payment } from "../types";
import { getContracts, refreshContracts } from "../storage";
import { apiListPayments } from "../api";
import { ContractDetailDialog } from "../ContractDetailDialog";

const STATUS_LABEL: Record<ContractStatus, string> = {
  active: "执行中",
  completed: "已完结",
  terminated: "已终止",
};

const STATUS_COLOR: Record<ContractStatus, string> = {
  active: "var(--c-primary)",
  completed: "#10b981",
  terminated: "var(--c-danger)",
};

function fmtMoney(n: number): string {
  return n.toLocaleString("zh-CN", { maximumFractionDigits: 0 });
}

function fmtDate(iso?: string | null): string {
  if (!iso) return "";
  return iso.slice(0, 10);
}

function normalize(s: string): string {
  return s.replace(/　/g, " ").trim().toLowerCase();
}

export function ContractsScreen() {
  const [contracts, setContracts] = useState<Contract[]>(getContracts());
  const [filter, setFilter] = useState<ContractStatus | "all">("active");
  const [query, setQuery] = useState("");
  const [viewing, setViewing] = useState<Contract | null>(null);
  const [paidByContract, setPaidByContract] = useState<Record<string, number>>({});

  useEffect(() => {
    void refreshContracts();
    const onChange = () => {
      setContracts(getContracts());
      setViewing((v) =>
        v ? getContracts().find((c) => c.id === v.id) ?? null : null,
      );
    };
    window.addEventListener("doudou:contracts_changed", onChange);
    return () => window.removeEventListener("doudou:contracts_changed", onChange);
  }, []);

  // 一次拉所有 payments, 按 contract_id 聚合 (用于顶部应收概览 + 列表卡片)
  useEffect(() => {
    apiListPayments()
      .then((ps: Payment[]) => {
        const m: Record<string, number> = {};
        for (const p of ps) m[p.contract_id] = (m[p.contract_id] || 0) + p.amount;
        setPaidByContract(m);
      })
      .catch(console.warn);
  }, [contracts]);

  const counts = useMemo(() => {
    const c: Record<string, number> = { all: contracts.length };
    for (const x of contracts) c[x.status] = (c[x.status] || 0) + 1;
    return c;
  }, [contracts]);

  // 应收账款简化版: 仅 active 合同
  const summary = useMemo(() => {
    let total = 0;
    let paid = 0;
    for (const c of contracts) {
      if (c.status !== "active") continue;
      total += c.total_amount;
      paid += paidByContract[c.id] || 0;
    }
    return { total, paid, due: total - paid };
  }, [contracts, paidByContract]);

  const filtered = useMemo(() => {
    let list = contracts;
    if (filter !== "all") list = list.filter((c) => c.status === filter);
    const qs = normalize(query);
    if (qs)
      list = list.filter((c) =>
        normalize(`${c.customer_name} ${c.contract_no ?? ""}`).includes(qs),
      );
    return list;
  }, [contracts, filter, query]);

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">合同</h1>
          <div className="page-subtitle">
            执行中 {counts.active || 0} · 完结 {counts.completed || 0}
          </div>
        </div>
      </div>

      {/* 应收账款卡 */}
      {contracts.length > 0 && (
        <div
          className="cost-card"
          style={{
            background:
              "linear-gradient(135deg, #0a5fff 0%, #3b82f6 100%)",
            marginBottom: 16,
          }}
        >
          <div className="cost-label">待收账款 (执行中)</div>
          <div className="cost-amount">
            <span className="cost-int">¥{fmtMoney(summary.due)}</span>
          </div>
          <div className="cost-meta">
            <span className="badge">总额 ¥{fmtMoney(summary.total)}</span>
            <span style={{ opacity: 0.85 }}>已收 ¥{fmtMoney(summary.paid)}</span>
          </div>
        </div>
      )}

      <div style={{ position: "relative", marginBottom: 10 }}>
        <input
          type="search"
          className="search-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索客户名 / 合同号"
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
        {(["all", "active", "completed", "terminated"] as const).map((s) => (
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
            {s === "all" ? "全部" : STATUS_LABEL[s as ContractStatus]}{" "}
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
          {contracts.length === 0 ? (
            <>
              还没合同.
              <br />
              报价单状态切到「已签」后, 详情页有「转合同」按钮.
            </>
          ) : (
            "没有匹配的合同"
          )}
        </div>
      ) : (
        filtered.map((c) => {
          const paid = paidByContract[c.id] || 0;
          const due = c.total_amount - paid;
          return (
            <div
              key={c.id}
              className="card"
              style={{ marginBottom: 8, cursor: "pointer" }}
              onClick={() => setViewing(c)}
            >
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: 15,
                      fontWeight: 600,
                      marginBottom: 2,
                    }}
                  >
                    {c.customer_name}
                  </div>
                  <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
                    {c.contract_no || "无合同号"}
                    {" · "}
                    {fmtDate(c.signed_at)}
                    {" · "}
                    {fmtMoney(c.total_tons)} 吨
                  </div>
                </div>
                <div style={{ textAlign: "right" }}>
                  <div
                    style={{
                      fontSize: 14,
                      fontWeight: 700,
                      color:
                        due > 0 ? "var(--c-primary)" : "#10b981",
                    }}
                  >
                    ¥{fmtMoney(c.total_amount)}
                  </div>
                  <div
                    style={{
                      fontSize: 11,
                      fontWeight: 600,
                      color: STATUS_COLOR[c.status],
                      marginTop: 2,
                    }}
                  >
                    {STATUS_LABEL[c.status]}
                    {c.status === "active" && due > 0 && (
                      <span style={{ marginLeft: 4, color: "var(--c-text-3)", fontWeight: 400 }}>
                        待收 ¥{fmtMoney(due)}
                      </span>
                    )}
                  </div>
                </div>
              </div>
            </div>
          );
        })
      )}

      {viewing && (
        <ContractDetailDialog
          contract={viewing}
          onClose={() => setViewing(null)}
        />
      )}
    </>
  );
}
