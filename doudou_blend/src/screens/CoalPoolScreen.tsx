/**
 * 屏 2 - 煤池
 * 列出 master 里 73+ 种煤, 状态分组, 显示化验值 + 价格.
 * 当前版本: 只读展示. 编辑功能后续迭代加 (走 user_overrides 表).
 */
import { useEffect, useState } from "react";
import { loadMaster } from "../master_loader";
import { INDICATOR_LABEL } from "../types";
import type { CoalMaster, CoalStatus, MasterCoalEntry } from "../types";

const STATUS_LABEL: Record<CoalStatus, string> = {
  verified: "主力煤",
  active: "备选煤",
  draft: "待核实",
  incomplete: "数据未录入",
  archived: "已停用",
};

const STATUS_ORDER: CoalStatus[] = [
  "verified",
  "active",
  "draft",
  "incomplete",
  "archived",
];

export function CoalPoolScreen() {
  const [master, setMaster] = useState<CoalMaster | null>(null);
  const [filter, setFilter] = useState<CoalStatus | "all">("all");

  useEffect(() => {
    loadMaster().then(setMaster).catch(console.error);
  }, []);

  if (!master) {
    return <div className="loading">加载中...</div>;
  }

  const counts: Record<string, number> = {};
  for (const c of master.coals) {
    counts[c.status] = (counts[c.status] || 0) + 1;
  }

  const filtered =
    filter === "all"
      ? master.coals
      : master.coals.filter((c) => c.status === filter);

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">煤池</h1>
          <div className="page-subtitle">
            共 {master.coals.length} 种煤 (master {master.version})
          </div>
        </div>
      </div>

      {/* 状态过滤器 */}
      <div
        style={{
          display: "flex",
          gap: 6,
          overflowX: "auto",
          marginBottom: 12,
          paddingBottom: 4,
        }}
      >
        <FilterChip
          active={filter === "all"}
          onClick={() => setFilter("all")}
          label={`全部 ${master.coals.length}`}
        />
        {STATUS_ORDER.map((s) => (
          <FilterChip
            key={s}
            active={filter === s}
            onClick={() => setFilter(s)}
            label={`${STATUS_LABEL[s]} ${counts[s] || 0}`}
          />
        ))}
      </div>

      {filtered.map((coal) => (
        <CoalCard key={coal.name} coal={coal} />
      ))}
    </>
  );
}

function FilterChip({
  active,
  onClick,
  label,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        flexShrink: 0,
        padding: "6px 12px",
        borderRadius: 999,
        fontSize: 12,
        fontWeight: 600,
        background: active ? "var(--c-primary)" : "var(--c-card)",
        color: active ? "white" : "var(--c-text-2)",
        boxShadow: "var(--shadow-sm)",
      }}
    >
      {label}
    </button>
  );
}

function CoalCard({ coal }: { coal: MasterCoalEntry }) {
  const PROP_ORDER_TOP = ["S", "A", "V", "G"];
  const propOrder2 = ["M", "petro", "Y", "CSR"];

  const cif =
    coal.fob != null && coal.frt != null ? coal.fob + coal.frt : null;

  return (
    <div className="coal-card">
      <div className="coal-row">
        <div>
          <div className="coal-name">{coal.name}</div>
          <div className="coal-region">
            {coal.region || "未知产地"}
            {coal.coal_type ? ` · ${coal.coal_type}` : ""}
          </div>
        </div>
        <div>
          {cif != null ? (
            <>
              <div className="coal-price">¥{cif}</div>
              <div className="coal-price-detail">
                {coal.fob} + 运费 {coal.frt}
              </div>
            </>
          ) : (
            <span className={`status-pill status-${coal.status}`}>
              {STATUS_LABEL[coal.status]}
            </span>
          )}
        </div>
      </div>

      {/* 化验值 */}
      {PROP_ORDER_TOP.some((k) => coal.props[k] != null) && (
        <div className="coal-props">
          {PROP_ORDER_TOP.map((k) => {
            const v = coal.props[k];
            if (v == null)
              return (
                <div key={k} className="coal-prop empty">
                  <div className="coal-prop-label">{INDICATOR_LABEL[k]}</div>
                  <div className="coal-prop-value">—</div>
                </div>
              );
            return (
              <div key={k} className="coal-prop">
                <div className="coal-prop-label">{INDICATOR_LABEL[k]}</div>
                <div className="coal-prop-value">{v}</div>
              </div>
            );
          })}
        </div>
      )}

      {/* 第二行 (仅 verified 才有) */}
      {coal.status === "verified" && (
        <div className="coal-props">
          {propOrder2.map((k) => {
            const v = coal.props[k];
            if (v == null)
              return (
                <div key={k} className="coal-prop empty">
                  <div className="coal-prop-label">{INDICATOR_LABEL[k]}</div>
                  <div className="coal-prop-value">—</div>
                </div>
              );
            return (
              <div key={k} className="coal-prop">
                <div className="coal-prop-label">{INDICATOR_LABEL[k]}</div>
                <div className="coal-prop-value">{v}</div>
              </div>
            );
          })}
        </div>
      )}

      <div className="coal-status-row">
        <span className={`status-pill status-${coal.status}`}>
          {STATUS_LABEL[coal.status]}
        </span>
        {coal.note && (
          <span style={{ fontSize: 10, color: "var(--c-text-3)", marginLeft: 8 }}>
            {coal.note}
          </span>
        )}
      </div>
    </div>
  );
}
