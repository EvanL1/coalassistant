/**
 * 屏 2 - 煤池
 * 煤种列表 + 状态过滤 + 点击编辑 (CoalEditor)
 */
import { useEffect, useMemo, useState } from "react";
import {
  resolveCoalPool,
  type ResolvedCoal,
} from "../domain/resolvedCoal";
import { buildPriceAnchor } from "../domain/priceDrift";
import { loadMaster } from "../master_loader";
import { INDICATOR_LABEL } from "../types";
import type { CoalMaster, CoalStatus, MasterCoalEntry } from "../types";
import { CoalEditor } from "../CoalEditor";
import { NewCoalDialog } from "../NewCoalDialog";
import { PenaltyTemplateEditor } from "../PenaltyTemplateEditor";
import { getPenaltyTemplate, PENALTY_TEMPLATE_EVENT } from "../penaltyStorage";
import {
  enableAllCoals,
  getCoalPrefs,
  getUserCoals,
  normalizeCoalName,
  type CoalPrefs,
} from "../storage";

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
  const [userCoals, setUserCoals] = useState<MasterCoalEntry[]>([]);
  const [filter, setFilter] = useState<CoalStatus | "all" | "enabled" | "hidden">("all");
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<ResolvedCoal | null>(null);
  const [showNew, setShowNew] = useState(false);
  const [templateClauseCount, setTemplateClauseCount] = useState(
    () => getPenaltyTemplate()?.clauses.length ?? 0,
  );

  useEffect(() => {
    loadMaster().then(setMaster).catch(console.error);
    setPrefs(getCoalPrefs());
    setUserCoals(getUserCoals());

    const onPrefs = () => setPrefs(getCoalPrefs());
    const onUserCoals = () => setUserCoals(getUserCoals());
    const onTemplate = () =>
      setTemplateClauseCount(getPenaltyTemplate()?.clauses.length ?? 0);
    window.addEventListener("doudou:prefs_changed", onPrefs);
    window.addEventListener("doudou:user_coals_changed", onUserCoals);
    window.addEventListener(PENALTY_TEMPLATE_EVENT, onTemplate);
    return () => {
      window.removeEventListener("doudou:prefs_changed", onPrefs);
      window.removeEventListener("doudou:user_coals_changed", onUserCoals);
      window.removeEventListener(PENALTY_TEMPLATE_EVENT, onTemplate);
    };
  }, []);

  const allBaseCoals = useMemo<MasterCoalEntry[]>(
    () => (master ? [...userCoals, ...master.coals] : []),
    [master, userCoals],
  );

  // 卡片、筛选和求解器共用同一套有效值解析，用户新增煤排在前面。
  const allCoals = useMemo<ResolvedCoal[]>(
    () =>
      resolveCoalPool(master?.coals ?? [], userCoals, prefs, {
        anchor: buildPriceAnchor(prefs),
        masterUpdatedAt: master?.updated_at ?? null,
      }),
    [master, prefs, userCoals],
  );
  const masterCoalNames = useMemo(
    () =>
      new Set(
        (master?.coals ?? []).map((coal) =>
          normalizeCoalName(coal.name),
        ),
      ),
    [master],
  );

  if (!master) {
    return <div className="loading">加载中...</div>;
  }

  function isHidden(coal: ResolvedCoal): boolean {
    return coal.hidden;
  }
  function isEnabled(coal: ResolvedCoal): boolean {
    return coal.effectiveEnabled;
  }

  // 默认所有视图都过滤掉 hidden 煤 (只有 hidden filter 显示)
  const visibleCoals = allCoals.filter((c) => !isHidden(c));
  const hiddenCount = allCoals.length - visibleCoals.length;

  const counts: Record<string, number> = {};
  for (const c of visibleCoals) {
    counts[c.status] = (counts[c.status] || 0) + 1;
  }

  const byStatus =
    filter === "hidden"
      ? allCoals.filter(isHidden)
      : filter === "all"
      ? visibleCoals
      : filter === "enabled"
      ? visibleCoals.filter(isEnabled)
      : visibleCoals.filter((c) => c.status === filter);

  // 搜索: 大小写无关 + 全角空格容错, 匹配煤名 / 产地 / 煤类任一字段
  const q = normalizeCoalName(query);
  const filtered = q
    ? byStatus.filter((c) => {
        const hay = `${c.name} ${c.region ?? ""} ${c.coal_type ?? ""}`;
        return normalizeCoalName(hay).includes(q);
      })
    : byStatus;

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">煤池</h1>
          <div className="page-subtitle">
            共 {visibleCoals.length} 种煤
            {userCoals.length > 0 && ` (含新增 ${userCoals.length})`}
            {hiddenCount > 0 && ` · 隐藏 ${hiddenCount}`}
            {" · 今日启用 "}
            {visibleCoals.filter(isEnabled).length} 种
          </div>
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button
            onClick={() => enableAllCoals(visibleCoals.map((c) => c.name))}
            style={{
              padding: "6px 12px",
              borderRadius: 8,
              background: "var(--c-card)",
              color: "var(--c-text-2)",
              fontSize: 12,
              fontWeight: 600,
              boxShadow: "var(--shadow-sm)",
              whiteSpace: "nowrap",
            }}
          >
            全部启用
          </button>
          <button
            aria-label="新增煤种"
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
      </div>

      <details
        className="card"
        style={{ padding: "10px 12px", marginBottom: 10 }}
      >
        <summary
          style={{
            cursor: "pointer",
            fontSize: 12,
            fontWeight: 600,
            color: "var(--c-text-2)",
          }}
        >
          采购扣款模板 ·{" "}
          {templateClauseCount > 0 ? `${templateClauseCount} 条条款` : "未设置"}
        </summary>
        <PenaltyTemplateEditor />
      </details>

      <div style={{ position: "relative", marginBottom: 10 }}>
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索煤名 / 产地 / 煤类"
          className="search-input"
        />
        {query && (
          <button
            aria-label="清空搜索"
            onClick={() => setQuery("")}
            style={{
              position: "absolute",
              right: 10,
              top: "50%",
              transform: "translateY(-50%)",
              width: 22,
              height: 22,
              borderRadius: "50%",
              background: "var(--c-text-3)",
              color: "white",
              fontSize: 14,
              lineHeight: 1,
            }}
          >
            ×
          </button>
        )}
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
          label={`全部 ${visibleCoals.length}`}
        />
        <FilterChip
          active={filter === "enabled"}
          onClick={() => setFilter("enabled")}
          label={`已启用 ${visibleCoals.filter(isEnabled).length}`}
        />
        {STATUS_ORDER.map((s) => (
          <FilterChip
            key={s}
            active={filter === s}
            onClick={() => setFilter(s)}
            label={`${STATUS_LABEL[s]} ${counts[s] || 0}`}
          />
        ))}
        {hiddenCount > 0 && (
          <FilterChip
            active={filter === "hidden"}
            onClick={() => setFilter("hidden")}
            label={`已隐藏 ${hiddenCount}`}
          />
        )}
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
            ? `没找到匹配「${query}」的煤种`
            : "当前过滤条件下没有煤种"}
        </div>
      ) : (
        <div className="coal-card-grid">
          {filtered.map((coal, index) => (
            <CoalCard
              key={`${coal.origin}:${coal.name}:${index}`}
              coal={coal}
              onClick={() => setEditing(coal)}
            />
          ))}
        </div>
      )}

      {editing && (
        <CoalEditor
          coal={editing.base}
          isUserAdded={editing.origin === "user"}
          preservePrefOnDelete={
            editing.origin === "user" &&
            masterCoalNames.has(normalizeCoalName(editing.name))
          }
          masterUpdatedAt={master.updated_at}
          onClose={() => setEditing(null)}
        />
      )}

      {showNew && (
        <NewCoalDialog
          existing={allBaseCoals}
          onClose={() => setShowNew(false)}
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
  onClick,
}: {
  coal: ResolvedCoal;
  onClick: () => void;
}) {
  const PROP_ORDER_TOP = ["S", "A", "V", "G"];
  const PROP_ORDER_BOTTOM = ["M", "petro", "Y", "CSR"];
  const enabled = coal.effectiveEnabled;
  const readinessText: Record<ResolvedCoal["readiness"], string> = {
    ready: "● 启用中",
    disabled: "○ 停用 · 点击编辑",
    hidden: "○ 已隐藏 · 点击恢复",
    missing_fob: "● 已启用 · 缺出厂价，未参与求解",
    missing_frt: "● 已启用 · 缺运费，未参与求解",
    duplicate_name: "名称冲突 · 未参与求解",
    invalid_override: "● 已启用 · 修改数据无效",
  };

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
          <div className="coal-name">
            {coal.name}
            {coal.hasOverrides && (
              <span style={{ color: "var(--c-primary)", fontSize: 11 }}>
                {" "}· 已修改
              </span>
            )}
          </div>
          <div className="coal-region">
            {coal.region || "未知产地"}
            {coal.coal_type ? ` · ${coal.coal_type}` : ""}
          </div>
        </div>
        <div>
          {coal.cif != null ? (
            <>
              <div className="coal-price">¥{coal.cif}</div>
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

      {PROP_ORDER_BOTTOM.some((k) => coal.props[k] != null) && (
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
            color:
              coal.readiness === "ready"
                ? "var(--c-success)"
                : coal.effectiveEnabled
                  ? "var(--c-danger)"
                  : "var(--c-text-3)",
            fontWeight: 600,
          }}
        >
          {readinessText[coal.readiness]}
        </span>
      </div>
    </div>
  );
}
