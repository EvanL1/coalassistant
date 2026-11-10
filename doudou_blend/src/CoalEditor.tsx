/**
 * 煤编辑底部弹层.
 *
 * 入口: 点煤池里的煤卡 → 弹出
 * 功能:
 *   - 改 8 项化验值 (S/A/V/G/Y/petro/CSR/M)
 *   - 启用/停用
 *   - 不录价: 单煤真实报价拿不到, 煤价一律按质量估算 (见 domain/resolvedCoal.ts)
 *   - "重置为 master 默认" 按钮 (清除 user_overrides)
 */
import { useEffect, useState } from "react";
import { resolveCoal, type CoalOrigin } from "./domain/resolvedCoal";
import { guaranteeIssue } from "./penalty";
import { getPenaltyTemplate } from "./penaltyStorage";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "./types";
import { isPartialNumberInput, numberInputError, numberOrNull } from "./domain/numberInput";
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
  /** 旧数据与 Master 重名时，删除用户煤不能连带清掉共用的 Master 偏好。 */
  preservePrefOnDelete?: boolean;
  onClose: () => void;
  onSaved?: () => void;
}

interface FormState {
  enabled: boolean;
  props: Record<string, string>;
  /** 该煤采购合同的保证值 (买入侧): 达不到这个数供应商扣钱, 到厂价更低. */
  guarantees: Record<string, string>;
}

function toForm(
  coal: MasterCoalEntry,
  pref: CoalPref | null,
  origin: CoalOrigin,
): FormState {
  const resolved = resolveCoal(coal, pref, origin);
  return {
    enabled: resolved.requestedEnabled,
    props: Object.fromEntries(
      INDICATOR_ORDER.map((key) => [
        key,
        resolved.props[key] != null ? String(resolved.props[key]) : "",
      ]),
    ),
    guarantees: Object.fromEntries(
      INDICATOR_ORDER.map((key) => {
        const value = pref?.purchase_guarantees?.[key];
        return [key, value != null ? String(value) : ""];
      }),
    ),
  };
}

function parseNumOrNull(s: string): number | null {
  return numberOrNull(s, "indicator");
}

export function CoalEditor({
  coal,
  isUserAdded,
  preservePrefOnDelete,
  onClose,
  onSaved,
}: Props) {
  const origin: CoalOrigin = isUserAdded ? "user" : "master";
  const [pref, setPref] = useState<CoalPref | null>(getCoalPref(coal.name));
  const [form, setForm] = useState<FormState>(() =>
    toForm(coal, pref, origin),
  );
  // 采购扣款模板只在打开煤卡时读一次: 模板的编辑入口在煤池屏顶部, 与这个
  // 弹层不会同时开着.
  const [template] = useState(() => getPenaltyTemplate());
  const resolved = resolveCoal(coal, pref, origin);
  const isHidden = resolved.hidden;
  // 煤库标了停用的煤, 用户偏好里的启用拉不回来 (见 resolveCoal), 开关点了也白点.
  const archived = coal.status === "archived";
  const propErrors = Object.fromEntries(
    INDICATOR_ORDER.map((k) => [k, numberInputError(form.props[k], "indicator")]),
  );
  const guaranteeErrors = Object.fromEntries(
    INDICATOR_ORDER.map((k) => [k, numberInputError(form.guarantees[k], "indicator")]),
  );
  // 输到一半的中间态 (末尾逗号/小数点) 输入中不亮错; 失焦或点过保存后照实报.
  const [blurred, setBlurred] = useState<ReadonlySet<string>>(new Set());
  const [triedSave, setTriedSave] = useState(false);
  const markBlurred = (field: string) =>
    setBlurred((prev) => (prev.has(field) ? prev : new Set(prev).add(field)));
  const showError = (field: string, raw: string, error: string | null): string | null =>
    error != null && (triedSave || blurred.has(field) || !isPartialNumberInput(raw))
      ? error
      : null;
  const visiblePropError = (k: string) => showError(`prop:${k}`, form.props[k], propErrors[k]);
  const visibleGuaranteeError = (k: string) =>
    showError(`guarantee:${k}`, form.guarantees[k], guaranteeErrors[k]);
  const invalidFields = [
    ...INDICATOR_ORDER.filter((k) => propErrors[k]).map((k) => `${INDICATOR_LABEL[k]}化验值`),
    ...INDICATOR_ORDER.filter((k) => guaranteeErrors[k]).map((k) => `${INDICATOR_LABEL[k]}保证值`),
  ];
  const visibleInvalidFields = [
    ...INDICATOR_ORDER.filter((k) => visiblePropError(k)).map((k) => `${INDICATOR_LABEL[k]}化验值`),
    ...INDICATOR_ORDER.filter((k) => visibleGuaranteeError(k)).map((k) => `${INDICATOR_LABEL[k]}保证值`),
  ];

  // 锁住 body 滚动
  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  function save() {
    if (invalidFields.length > 0) {
      // 点保存才校验输到一半的中间态: 把所有错亮出来.
      setTriedSave(true);
      return;
    }
    const propsOverride: Record<string, number> = {};
    for (const k of INDICATOR_ORDER) {
      const newVal = parseNumOrNull(form.props[k]);
      const masterVal = coal.props[k];
      // 只存"跟 master 不一样"的值, 节省空间
      if (newVal != null && newVal !== masterVal) {
        propsOverride[k] = newVal;
      }
    }

    const guarantees: Record<string, number> = {};
    for (const k of INDICATOR_ORDER) {
      const value = parseNumOrNull(form.guarantees[k]);
      if (value != null) guarantees[k] = value;
    }

    setCoalPref(coal.name, {
      enabled: form.enabled,
      props_override: Object.keys(propsOverride).length > 0 ? propsOverride : undefined,
      // 清空某项保证值就得让它从 map 里消失: 留着 undefined 会被
      // mergePurchaseTerms 跳过(它只认有限数), 但 JSON 往返后行为一致, 干脆整
      // 份重写 —— 界面上 8 项全在, 这份 map 就是完整的真源.
      purchase_guarantees:
        Object.keys(guarantees).length > 0 ? guarantees : undefined,
    });
    onSaved?.();
    onClose();
  }

  function resetToMaster() {
    // 采购保证值没有 master 默认值(master 不含采购合同), 但它跟化验覆盖
    // 存在同一份 CoalPref 里, 重置会一并清掉 —— 得在确认框里说出来.
    const alsoDropped =
      filledGuarantees > 0 ? `\n已填的 ${filledGuarantees} 项采购保证值也会一并清掉。` : "";
    if (!confirm(`重置 ${coal.name} 的所有修改, 回到煤库默认值?${alsoDropped}`)) {
      return;
    }
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
  const filledGuarantees = INDICATOR_ORDER.filter(
    (k) => form.guarantees[k].trim() !== "",
  ).length;

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
              role="switch"
              aria-label="在配煤中启用"
              aria-checked={form.enabled}
              aria-disabled={archived}
              style={archived ? { opacity: 0.4, cursor: "not-allowed" } : undefined}
              onClick={() => {
                if (!archived) setForm({ ...form, enabled: !form.enabled });
              }}
            />
          </div>
          {archived && (
            <div style={{ fontSize: 11, color: "var(--c-text-3)", padding: "0 0 8px" }}>
              这种煤在煤库里已标为停用, 不能在配煤中启用
            </div>
          )}

          {/* 化验值 */}
          <div className="edit-section">
            <div className="edit-section-title">化验指标</div>
            {INDICATOR_ORDER.map((k) => (
              <div key={k}>
                <div className="edit-row">
                  <div className="edit-row-label">{INDICATOR_LABEL[k]}</div>
                  <input
                    type="text"
                    inputMode="decimal"
                    className="edit-input"
                    aria-label={`${INDICATOR_LABEL[k]}化验值`}
                    aria-invalid={visiblePropError(k) ? true : undefined}
                    value={form.props[k]}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        props: { ...form.props, [k]: e.target.value },
                      })
                    }
                    onBlur={() => markBlurred(`prop:${k}`)}
                    placeholder={
                      coal.props[k] != null ? String(coal.props[k]) : "未录入"
                    }
                  />
                </div>
                {visiblePropError(k) && (
                  <div className="edit-input-error">{visiblePropError(k)}</div>
                )}
              </div>
            ))}
          </div>

          {/* 采购保证值 (买入侧): 条款在煤池顶部的模板里, 保证值逐煤不同 */}
          <details className="edit-section">
            <summary
              className="edit-section-title"
              style={{ cursor: "pointer", listStyle: "revert" }}
            >
              采购保证值 · {filledGuarantees > 0 ? `已填 ${filledGuarantees} 项` : "未填"}
            </summary>
            <div style={{ fontSize: 10, color: "var(--c-text-3)", padding: "4px 0 8px" }}>
              这个煤的采购合同保证到多少。达不到保证值供应商扣钱，你少付，到厂价更低。
              扣多少由煤池顶部的「采购扣款模板」定。
            </div>
            {/*
              红字(error 级)只提示, 不挡保存 —— 与合同屏刻意不同。
              合同屏的条款自成一体, 填不对就该拦住; 这里的对错取决于模板里的
              拒收线, 用户完全可能先填保证值再去改模板, 拦住等于逼他先放弃这次
              修改(而这个弹层里还有化验值要存)。
            */}
            {INDICATOR_ORDER.map((k) => {
              const raw = form.guarantees[k];
              const issue =
                raw.trim() === "" || guaranteeErrors[k]
                  ? null
                  : guaranteeIssue({
                      template,
                      override: pref?.purchase_override,
                      indicator: k,
                      guarantee: parseNumOrNull(raw) ?? Number.NaN,
                    });
              return (
                <div key={k}>
                  <div className="edit-row">
                    <div className="edit-row-label">{INDICATOR_LABEL[k]}</div>
                    <input
                      type="text"
                      inputMode="decimal"
                      className="edit-input"
                      aria-label={`${INDICATOR_LABEL[k]}保证值`}
                      aria-invalid={visibleGuaranteeError(k) ? true : undefined}
                      value={raw}
                      onChange={(e) =>
                        setForm({
                          ...form,
                          guarantees: { ...form.guarantees, [k]: e.target.value },
                        })
                      }
                      onBlur={() => markBlurred(`guarantee:${k}`)}
                      placeholder="未填"
                    />
                  </div>
                  {visibleGuaranteeError(k) && (
                    <div className="edit-input-error">{visibleGuaranteeError(k)}</div>
                  )}
                  {issue && (
                    <div
                      style={{
                        fontSize: 10,
                        lineHeight: 1.5,
                        padding: "0 0 8px",
                        color:
                          issue.level === "error"
                            ? "var(--c-danger)"
                            : "var(--c-text-3)",
                      }}
                    >
                      {issue.message}
                    </div>
                  )}
                </div>
              );
            })}
          </details>

          {hasOverrides && (
            <button
              className="btn btn-secondary"
              style={{ width: "100%", color: "var(--c-danger)" }}
              onClick={resetToMaster}
            >
              重置为煤库默认值
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

        {visibleInvalidFields.length > 0 && (
          <div role="alert" className="edit-input-error" style={{ padding: "0 16px" }}>
            有 {visibleInvalidFields.length} 处没填对: {visibleInvalidFields.join("、")}，改好才能保存
          </div>
        )}
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
