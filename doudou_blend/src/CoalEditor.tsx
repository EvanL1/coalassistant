/**
 * 煤编辑底部弹层.
 *
 * 入口: 点煤池里的煤卡 → 弹出
 * 功能:
 *   - 改 8 项化验值 (S/A/V/G/Y/petro/CSR/M)
 *   - 改 FOB / FRT
 *   - 启用/停用
 *   - "重置为 master 默认" 按钮 (清除 user_overrides)
 */
import { useEffect, useState } from "react";
import { resolveCoal, type CoalOrigin } from "./domain/resolvedCoal";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "./types";
import type { MasterCoalEntry } from "./types";
import {
  getCoalPref,
  setCoalPref,
  clearCoalPref,
  recordCoalQuote,
  removeUserCoal,
  type CoalPref,
} from "./storage";

interface Props {
  coal: MasterCoalEntry;
  /** true = 用户自己新增的 (走 removeUserCoal 真删); false/undefined = master 煤 (走 hidden=true 软隐藏) */
  isUserAdded?: boolean;
  /** 旧数据与 Master 重名时，删除用户煤不能连带清掉共用的 Master 偏好。 */
  preservePrefOnDelete?: boolean;
  /** master 的 updated_at, 首次改价时用来给报价历史补起点 */
  masterUpdatedAt?: string | null;
  onClose: () => void;
  onSaved?: () => void;
}

interface FormState {
  enabled: boolean;
  fob: string;
  frt: string;
  /** 作为价格锚点参与推算 */
  isAnchor: boolean;
  props: Record<string, string>;
}

function toForm(
  coal: MasterCoalEntry,
  pref: CoalPref | null,
  origin: CoalOrigin,
): FormState {
  const resolved = resolveCoal(coal, pref, origin);
  return {
    enabled: resolved.requestedEnabled,
    fob: resolved.fob != null ? String(resolved.fob) : "",
    frt: resolved.frt != null ? String(resolved.frt) : "",
    isAnchor: pref?.is_price_anchor === true,
    props: Object.fromEntries(
      INDICATOR_ORDER.map((key) => [
        key,
        resolved.props[key] != null ? String(resolved.props[key]) : "",
      ]),
    ),
  };
}

/** 距今天数; 日期不合法返回 null. */
function daysSince(date: string): number | null {
  const t = Date.parse(`${date}T00:00:00`);
  if (!Number.isFinite(t)) return null;
  return Math.max(0, Math.round((Date.now() - t) / 86_400_000));
}

function parseNumOrNull(s: string): number | null {
  if (!s.trim()) return null;
  const n = parseFloat(s);
  return Number.isFinite(n) ? n : null;
}

export function CoalEditor({
  coal,
  isUserAdded,
  preservePrefOnDelete,
  masterUpdatedAt,
  onClose,
  onSaved,
}: Props) {
  const origin: CoalOrigin = isUserAdded ? "user" : "master";
  const [pref, setPref] = useState<CoalPref | null>(getCoalPref(coal.name));
  const [form, setForm] = useState<FormState>(() =>
    toForm(coal, pref, origin),
  );
  const resolved = resolveCoal(coal, pref, origin);
  const isHidden = resolved.hidden;
  const quotedAt = resolved.fob_quoted_at ?? masterUpdatedAt ?? null;
  const staleDays = quotedAt ? daysSince(quotedAt) : null;
  // 一个点算不出涨跌. 锚点开着却只有 0~1 条报价时会静默不生效, 必须说出来.
  const quoteCount = pref?.fob_history?.length ?? 0;
  const anchorNotReady = form.isAnchor && quoteCount < 2;

  // 锁住 body 滚动
  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  function save() {
    const propsOverride: Record<string, number> = {};
    for (const k of INDICATOR_ORDER) {
      const newVal = parseNumOrNull(form.props[k]);
      const masterVal = coal.props[k];
      // 只存"跟 master 不一样"的值, 节省空间
      if (newVal != null && newVal !== masterVal) {
        propsOverride[k] = newVal;
      }
    }
    const fobNum = parseNumOrNull(form.fob);
    const frtNum = parseNumOrNull(form.frt);

    setCoalPref(coal.name, {
      enabled: form.enabled,
      fob_override: fobNum !== coal.fob ? fobNum : null,
      frt_override: frtNum !== coal.frt ? frtNum : null,
      is_price_anchor: form.isAnchor,
      props_override: Object.keys(propsOverride).length > 0 ? propsOverride : undefined,
    });
    // 出厂价变了才记一次报价. 报价历史是漂移推算唯一的数据来源, 没有它锚点算不出比例.
    if (fobNum != null && fobNum !== resolved.fob) {
      const seedDate = pref?.fob_quoted_at ?? masterUpdatedAt ?? null;
      const seed =
        resolved.fob != null && seedDate
          ? { date: seedDate, fob: resolved.fob }
          : null;
      recordCoalQuote(coal.name, fobNum, seed);
    }
    onSaved?.();
    onClose();
  }

  function resetToMaster() {
    if (!confirm(`重置 ${coal.name} 的所有修改, 回到 master 默认值?`)) return;
    clearCoalPref(coal.name);
    setPref(null);
    setForm(toForm(coal, null, origin));
  }

  function removeCoal() {
    if (isUserAdded) {
      if (!confirm(`彻底删除「${coal.name}」？\n用户自定义煤, 数据无法恢复.`)) return;
      removeUserCoal(coal.name, preservePrefOnDelete);
    } else {
      if (!confirm(`隐藏「${coal.name}」？\n之后不再在煤池和求解器中出现. 可在「已隐藏」筛选里找回.`)) return;
      setCoalPref(coal.name, { hidden: true });
    }
    onSaved?.();
    onClose();
  }

  function unhideCoal() {
    setCoalPref(coal.name, { hidden: false });
    setPref(getCoalPref(coal.name));
    onSaved?.();
  }

  const hasOverrides = resolved.hasOverrides;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal-sheet" onClick={(e) => e.stopPropagation()}>
        <div className="modal-handle" />

        <div className="modal-header">
          <div>
            <div className="modal-title">{coal.name}</div>
            <div className="modal-subtitle">
              {coal.region || "未知"}
              {coal.coal_type ? ` · ${coal.coal_type}` : ""}
              {hasOverrides && (
                <span style={{ color: "var(--c-primary)", marginLeft: 6 }}>
                  · 已修改
                </span>
              )}
            </div>
          </div>
          <button className="modal-close" onClick={onClose}>
            ×
          </button>
        </div>

        <div className="modal-body">
          {/* 启用开关 */}
          <div className="edit-row">
            <div className="edit-row-label">在配煤中启用</div>
            <div
              className={`toggle ${form.enabled ? "on" : ""}`}
              onClick={() => setForm({ ...form, enabled: !form.enabled })}
            />
          </div>

          {/* 价格 */}
          <div className="edit-section">
            <div className="edit-section-title">价格</div>
            <div className="edit-row">
              <div className="edit-row-label">出厂价 FOB</div>
              <input
                type="number"
                inputMode="decimal"
                className="edit-input"
                value={form.fob}
                onChange={(e) => setForm({ ...form, fob: e.target.value })}
                placeholder="元/吨"
              />
            </div>
            {quotedAt && (
              <div
                style={{
                  fontSize: 11,
                  padding: "0 0 8px",
                  color:
                    staleDays != null && staleDays > 30
                      ? "var(--c-danger)"
                      : "var(--c-text-3)",
                }}
              >
                上次录价 {quotedAt}
                {staleDays != null && ` · ${staleDays} 天前`}
                {staleDays != null && staleDays > 30 && " · 建议复核"}
              </div>
            )}
            <div className="edit-row">
              <div className="edit-row-label">
                作为价格锚点
                <div
                  style={{
                    fontSize: 11,
                    fontWeight: 400,
                    color: "var(--c-text-3)",
                    marginTop: 2,
                  }}
                >
                  用它的涨跌比例推算其他未更新的煤
                </div>
              </div>
              <div
                className={`toggle ${form.isAnchor ? "on" : ""}`}
                onClick={() => setForm({ ...form, isAnchor: !form.isAnchor })}
              />
            </div>
            {anchorNotReady && (
              <div
                style={{
                  fontSize: 11,
                  padding: "0 0 8px",
                  color: "var(--c-warning, #f59e0b)",
                }}
              >
                锚点还不能用: 一个价格点算不出涨跌。改一次出厂价并保存,
                系统会把旧价当起点, 从此开始推算。
              </div>
            )}
            <div className="edit-row">
              <div className="edit-row-label">运费 FRT</div>
              <input
                type="number"
                inputMode="decimal"
                className="edit-input"
                value={form.frt}
                onChange={(e) => setForm({ ...form, frt: e.target.value })}
                placeholder="元/吨"
              />
            </div>
            <div className="edit-row" style={{ background: "var(--c-bg)" }}>
              <div className="edit-row-label">到厂价 CIF</div>
              <div style={{ fontWeight: 700, color: "var(--c-primary)" }}>
                ¥
                {(
                  (parseNumOrNull(form.fob) ?? 0) +
                  (parseNumOrNull(form.frt) ?? 0)
                ).toFixed(2)}
              </div>
            </div>
          </div>

          {/* 化验值 */}
          <div className="edit-section">
            <div className="edit-section-title">化验指标</div>
            {INDICATOR_ORDER.map((k) => (
              <div className="edit-row" key={k}>
                <div className="edit-row-label">{INDICATOR_LABEL[k]}</div>
                <input
                  type="number"
                  inputMode="decimal"
                  className="edit-input"
                  value={form.props[k]}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      props: { ...form.props, [k]: e.target.value },
                    })
                  }
                  placeholder={
                    coal.props[k] != null ? String(coal.props[k]) : "未录入"
                  }
                />
              </div>
            ))}
          </div>

          {hasOverrides && (
            <button
              className="btn btn-secondary"
              style={{ width: "100%", color: "var(--c-danger)" }}
              onClick={resetToMaster}
            >
              重置为 master 默认值
            </button>
          )}

          {/* 危险区: 隐藏 / 删除 */}
          <div style={{ marginTop: 12, borderTop: "1px solid var(--c-border)", paddingTop: 12 }}>
            {isHidden ? (
              <button
                className="btn btn-secondary"
                style={{ width: "100%" }}
                onClick={unhideCoal}
              >
                取消隐藏 · 重新显示
              </button>
            ) : (
              <button
                className="btn btn-secondary"
                style={{ width: "100%", color: "var(--c-danger)", borderColor: "var(--c-danger)" }}
                onClick={removeCoal}
              >
                {isUserAdded ? "彻底删除此煤" : "隐藏此煤"}
              </button>
            )}
          </div>
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            取消
          </button>
          <button className="btn btn-primary" onClick={save}>
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
