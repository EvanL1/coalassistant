/**
 * 客户库 - Pre 合同流程 Phase 1
 * 列表 + 搜索 + 新增/编辑/删除
 */
import { useEffect, useMemo, useState } from "react";
import type { Customer } from "../types";
import {
  getCustomers,
  refreshCustomers,
  removeCustomer,
} from "../storage";
import { CustomerDialog } from "../CustomerDialog";

function normalize(s: string): string {
  return s.replace(/　/g, " ").trim().toLowerCase();
}

export function CustomersScreen() {
  const [customers, setCustomers] = useState<Customer[]>(getCustomers());
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<Customer | null>(null);
  const [showNew, setShowNew] = useState(false);

  useEffect(() => {
    void refreshCustomers();
    const onChange = () => setCustomers(getCustomers());
    window.addEventListener("doudou:customers_changed", onChange);
    return () => window.removeEventListener("doudou:customers_changed", onChange);
  }, []);

  const filtered = useMemo(() => {
    const q = normalize(query);
    if (!q) return customers;
    return customers.filter((c) => {
      const hay = `${c.name} ${c.contact ?? ""} ${c.phone ?? ""} ${c.note ?? ""}`;
      return normalize(hay).includes(q);
    });
  }, [customers, query]);

  async function onDelete(c: Customer) {
    if (!confirm(`删除客户「${c.name}」? 已生成的报价单不会删除.`)) return;
    try {
      await removeCustomer(c.id);
    } catch (e) {
      alert(`删除失败: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">客户</h1>
          <div className="page-subtitle">共 {customers.length} 个客户</div>
        </div>
        <button
          aria-label="新增客户"
          onClick={() => setShowNew(true)}
          style={{
            width: 40,
            height: 40,
            borderRadius: "50%",
            background: "var(--c-primary)",
            color: "white",
            fontSize: 22,
            fontWeight: 600,
            lineHeight: 1,
            boxShadow: "var(--shadow-sm)",
          }}
        >
          +
        </button>
      </div>

      <div style={{ position: "relative", marginBottom: 12 }}>
        <input
          type="search"
          className="search-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索 客户名 / 联系人 / 电话"
        />
      </div>

      {filtered.length === 0 ? (
        <div
          style={{
            textAlign: "center",
            color: "var(--c-text-3)",
            fontSize: 13,
            padding: "32px 16px",
          }}
        >
          {query
            ? `没找到匹配「${query}」的客户`
            : "还没有客户. 点右上 + 录第一个."}
        </div>
      ) : (
        filtered.map((c) => (
          <div
            key={c.id}
            className="card"
            style={{ marginBottom: 8, cursor: "pointer" }}
            onClick={() => setEditing(c)}
          >
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 2 }}>
                  {c.name}
                </div>
                <div style={{ fontSize: 12, color: "var(--c-text-3)" }}>
                  {c.contact && <span>{c.contact}</span>}
                  {c.contact && c.phone && <span> · </span>}
                  {c.phone && <span>{c.phone}</span>}
                </div>
                {c.note && (
                  <div style={{ fontSize: 12, color: "var(--c-text-2)", marginTop: 4 }}>
                    {c.note}
                  </div>
                )}
              </div>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  void onDelete(c);
                }}
                style={{
                  color: "var(--c-text-3)",
                  fontSize: 12,
                  padding: "4px 8px",
                }}
              >
                删除
              </button>
            </div>
          </div>
        ))
      )}

      {showNew && (
        <CustomerDialog editing={null} onClose={() => setShowNew(false)} />
      )}
      {editing && (
        <CustomerDialog
          editing={editing}
          onClose={() => setEditing(null)}
        />
      )}
    </>
  );
}
