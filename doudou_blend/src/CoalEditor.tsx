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
import { INDICATOR_LABEL, INDICATOR_ORDER } from "./types";
import type { MasterCoalEntry } from "./types";
import {
  getCoalPref,
  setCoalPref,
  clearCoalPref,
  removeUserCoal,
  type CoalPref,
} from "./storage";

interface Props {
  coal: MasterCoalEntry;
  /** true = 用户自己新增的 (走 removeUserCoal 真删); false/undefined = master 煤 (走 hidden=true 软隐藏) */
  isUserAdded?: boolean;
  onClose: () => void;
  onSaved?: () => void;
}

interface FormState {
  enabled: boolean;
  fob: string;
  frt: string;
  props: Record<string, string>;
}

function toForm(coal: MasterCoalEntry, pref: CoalPref | null): FormState {
  return {
    enabled: pref?.enabled ?? coal.status === "verified",
    fob: String(pref?.fob_override ?? coal.fob ?? ""),
    frt: String(pref?.frt_override ?? coal.frt ?? ""),
    props: Object.fromEntries(
      INDICATOR_ORDER.map((k) => {
        const override = pref?.props_override?.[k];
        const master = coal.props[k];
        const v = override ?? master;
        return [k, v != null ? String(v) : ""];
      })
    ),
  };
}

function parseNumOrNull(s: string): number | null {
  if (!s.trim()) return null;
  const n = parseFloat(s);
  return Number.isFinite(n) ? n : null;
}

export function CoalEditor({ coal, isUserAdded, onClose, onSaved }: Props) {
  const [pref, setPref] = useState<CoalPref | null>(getCoalPref(coal.name));
  const [form, setForm] = useState<FormState>(() => toForm(coal, pref));
  const isHidden = pref?.hidden === true;

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
      props_override: Object.keys(propsOverride).length > 0 ? propsOverride : undefined,
    });
    onSaved?.();
    onClose();
  }

  function resetToMaster() {
    if (!confirm(`重置 ${coal.name} 的所有修改, 回到 master 默认值?`)) return;
    clearCoalPref(coal.name);
    setPref(null);
    setForm(toForm(coal, null));
  }

  function removeCoal() {
    if (isUserAdded) {
      if (!confirm(`彻底删除「${coal.name}」？\n用户自定义煤, 数据无法恢复.`)) return;
      removeUserCoal(coal.name);
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

  const hasOverrides =
    pref != null &&
    (pref.fob_override != null ||
      pref.frt_override != null ||
      (pref.props_override && Object.keys(pref.props_override).length > 0));

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
