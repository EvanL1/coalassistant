/**
 * 屏 2 - 煤池
 * 73+ 煤列表 + 状态过滤 + 点击编辑 (CoalEditor)
 */
import { useEffect, useState } from "react";
import { loadMaster } from "../master_loader";
import { INDICATOR_LABEL } from "../types";
import type { CoalMaster, CoalStatus, MasterCoalEntry } from "../types";
import { CoalEditor } from "../CoalEditor";
import { getCoalPrefs, type CoalPrefs } from "../storage";

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
  const [prefs, setPrefs] = useState<CoalPrefs>({});
  const [filter, setFilter] = useState<CoalStatus | "all" | "enabled">("all");
  const [editing, setEditing] = useState<MasterCoalEntry | null>(null);

  useEffect(() => {
    loadMaster().then(setMaster).catch(console.error);
    setPrefs(getCoalPrefs());

    // 监听 prefs 变化
    const onChange = () => setPrefs(getCoalPrefs());
    window.addEventListener("doudou:prefs_changed", onChange);
    return () => window.removeEventListener("doudou:prefs_changed", onChange);
  }, []);

  if (!master) {
    return <div className="loading">加载中...</div>;
  }

  const counts: Record<string, number> = {};
  for (const c of master.coals) {
    counts[c.status] = (counts[c.status] || 0) + 1;
  }
  function isEnabled(coal: MasterCoalEntry): boolean {
    const p = prefs[coal.name];
    if (p?.enabled != null) return p.enabled;
    // 默认: verified 启用, 其他停用
    return coal.status === "verified";
  }

  const filtered =
    filter === "all"
      ? master.coals
      : filter === "enabled"
      ? master.coals.filter(isEnabled)
      : master.coals.filter((c) => c.status === filter);

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">煤池</h1>
          <div className="page-subtitle">
            共 {master.coals.length} 种煤 · 今日启用{" "}
            {master.coals.filter(isEnabled).length} 种
          </div>
        </div>
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
        <FilterChip
          active={filter === "all"}
          onClick={() => setFilter("all")}
          label={`全部 ${master.coals.length}`}
        />
        <FilterChip
          active={filter === "enabled"}
          onClick={() => setFilter("enabled")}
          label={`已启用 ${master.coals.filter(isEnabled).length}`}
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
        <CoalCard
          key={coal.name}
          coal={coal}
          enabled={isEnabled(coal)}
          onClick={() => setEditing(coal)}
        />
      ))}

      {editing && (
        <CoalEditor
          coal={editing}
          onClose={() => setEditing(null)}
        />
      )}
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

function CoalCard({
  coal,
  enabled,
  onClick,
}: {
  coal: MasterCoalEntry;
  enabled: boolean;
  onClick: () => void;
}) {
  const PROP_ORDER_TOP = ["S", "A", "V", "G"];
  const PROP_ORDER_BOTTOM = ["M", "petro", "Y", "CSR"];

  const cif =
    coal.fob != null && coal.frt != null ? coal.fob + coal.frt : null;

  return (
    <div
      className="coal-card"
      onClick={onClick}
      style={{
        cursor: "pointer",
        opacity: enabled ? 1 : 0.55,
      }}
    >
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
                单价 {coal.fob} + 运费 {coal.frt}
              </div>
            </>
          ) : (
            <span className={`status-pill status-${coal.status}`}>
              {STATUS_LABEL[coal.status]}
            </span>
          )}
        </div>
      </div>

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

      {coal.status === "verified" && (
        <div className="coal-props">
          {PROP_ORDER_BOTTOM.map((k) => {
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
        <span
          style={{
            fontSize: 11,
            color: enabled ? "var(--c-success)" : "var(--c-text-3)",
            fontWeight: 600,
          }}
        >
          {enabled ? "● 启用中" : "○ 停用 · 点击编辑"}
        </span>
      </div>
    </div>
  );
}
