/**
 * 屏 3 - 合同约束
 * 8 项 spec 的 min/max 可编辑. 存 localStorage.
 * "重置为默认" 恢复 master 自带的默认合同.
 */
import { useEffect, useState } from "react";
import { loadMaster } from "../master_loader";
import {
  getUserContract,
  setUserContract,
  clearUserContract,
} from "../storage";
import { INDICATOR_LABEL } from "../types";
import type {
  AcceptanceMode,
  CoalMaster,
  Enforcement,
  Spec,
} from "../types";

const ACCEPTANCE_OPTIONS: ReadonlyArray<{
  value: AcceptanceMode;
  label: string;
}> = [
  { value: "Raw", label: "按原值" },
  { value: "Truncate", label: "截断判定" },
  { value: "Round", label: "四舍五入" },
];

const ENFORCEMENT_OPTIONS: ReadonlyArray<{
  value: Enforcement;
  label: string;
}> = [
  { value: "Hard", label: "硬约束" },
  { value: "Soft", label: "软约束" },
  { value: "Advisory", label: "仅提示" },
];

interface FormSpec {
  indicator: string;
  direction: "Upper" | "Lower" | "Range";
  min: string;
  max: string;
  enabled: boolean;
  margin: string;
  acceptanceMode: AcceptanceMode;
  decimals: string;
  tolerance: string;
  enforcement: Enforcement;
}

function specToForm(s: Spec): FormSpec {
  const legacyAcceptance: AcceptanceMode =
    s.indicator === "petro" ? "Raw" : "Truncate";
  return {
    indicator: s.indicator,
    direction: s.direction,
    min: s.min != null ? String(s.min) : "",
    max: s.max != null ? String(s.max) : "",
    enabled: s.enabled !== false,
    margin: s.margin != null ? String(s.margin) : "",
    acceptanceMode: s.acceptance?.mode ?? legacyAcceptance,
    decimals:
      s.acceptance?.decimals != null
        ? String(s.acceptance.decimals)
        : legacyAcceptance === "Raw"
          ? ""
          : "1",
    tolerance:
      s.acceptance?.tolerance != null ? String(s.acceptance.tolerance) : "",
    enforcement: s.enforcement ?? "Hard",
  };
}

function optionalNumber(value: string): number | null {
  if (value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function formToSpec(f: FormSpec): Spec {
  const min = optionalNumber(f.min);
  const max = optionalNumber(f.max);
  const margin = optionalNumber(f.margin);
  const decimals = optionalNumber(f.decimals);
  const tolerance = optionalNumber(f.tolerance);
  return {
    indicator: f.indicator,
    direction: f.direction,
    min,
    max,
    enabled: f.enabled,
    margin: margin == null ? null : Math.max(0, margin),
    acceptance: {
      mode: f.acceptanceMode,
      decimals:
        f.acceptanceMode === "Raw"
          ? null
          : Math.min(6, Math.max(0, Math.trunc(decimals ?? 1))),
      tolerance: Math.max(0, tolerance ?? 0),
    },
    enforcement: f.enforcement,
  };
}

export function ContractScreen() {
  const [master, setMaster] = useState<CoalMaster | null>(null);
  const [form, setForm] = useState<FormSpec[]>([]);
  const [savedFlag, setSavedFlag] = useState(false);

  useEffect(() => {
    loadMaster().then((m) => {
      setMaster(m);
      const userOverride = getUserContract();
      const specs = userOverride ?? m.default_contract.specs;
      setForm(specs.map(specToForm));
    });
  }, []);

  if (!master) return <div className="loading">加载中...</div>;

  function updateSpec(i: number, patch: Partial<FormSpec>) {
    setForm((prev) => {
      const next = [...prev];
      next[i] = { ...next[i], ...patch };
      return next;
    });
  }

  function save() {
    const specs = form.map(formToSpec);
    setUserContract(specs);
    setSavedFlag(true);
    setTimeout(() => setSavedFlag(false), 2000);
  }

  function reset() {
    if (!confirm("重置合同为 master 默认值?")) return;
    clearUserContract();
    setForm(master!.default_contract.specs.map(specToForm));
  }

  const isCustom = getUserContract() != null;

  return (
    <>
      <div className="page-header">
        <div>
          <h1 className="page-title">合同约束</h1>
          <div className="page-subtitle">
            {isCustom ? "用户自定义" : master.default_contract.name}
          </div>
        </div>
      </div>

      <div className="card" style={{ padding: 0 }}>
        {form.map((f, i) => (
          <SpecRow
            key={f.indicator}
            spec={f}
            onChange={(patch) => updateSpec(i, patch)}
            isLast={i === form.length - 1}
          />
        ))}
      </div>

      <div className="action-row">
        <button className="btn btn-secondary" onClick={reset}>
          重置默认
        </button>
        <button className="btn btn-primary" onClick={save}>
          {savedFlag ? "✓ 已保存" : "保存合同"}
        </button>
      </div>

      <div
        style={{
          fontSize: 11,
          color: "var(--c-text-3)",
          marginTop: 12,
          padding: "0 4px",
        }}
      >
        提示: 截断/四舍五入决定边界值如何判定；约束安全余量用于主动收紧求解边界。
      </div>
    </>
  );
}

function SpecRow({
  spec,
  onChange,
  isLast,
}: {
  spec: FormSpec;
  onChange: (patch: Partial<FormSpec>) => void;
  isLast: boolean;
}) {
  const label = INDICATOR_LABEL[spec.indicator] || spec.indicator;
  const enforcementLabel =
    ENFORCEMENT_OPTIONS.find((option) => option.value === spec.enforcement)
      ?.label ?? spec.enforcement;
  const acceptanceLabel =
    ACCEPTANCE_OPTIONS.find(
      (option) => option.value === spec.acceptanceMode,
    )?.label ?? spec.acceptanceMode;

  return (
    <div
      style={{
        padding: "12px 14px",
        borderBottom: isLast ? "none" : "1px solid var(--c-border)",
        opacity: spec.enabled ? 1 : 0.5,
      }}
    >
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          marginBottom: 8,
        }}
      >
        <div>
          <div style={{ fontSize: 15, fontWeight: 600 }}>{label}</div>
          <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
            {spec.direction === "Upper"
              ? "上限 (越低越好)"
              : spec.direction === "Lower"
              ? "下限 (越高越好)"
              : "目标范围"}
          </div>
        </div>
        <div
          className={`toggle ${spec.enabled ? "on" : ""}`}
          onClick={() => onChange({ enabled: !spec.enabled })}
        />
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: 8,
        }}
      >
        {(spec.direction === "Lower" || spec.direction === "Range") && (
          <NumberInput
            label="≥"
            value={spec.min}
            disabled={!spec.enabled}
            onChange={(v) => onChange({ min: v })}
          />
        )}
        {spec.direction === "Upper" && (
          <NumberInput
            label="任意值 (无下限)"
            value="—"
            disabled
            onChange={() => {}}
          />
        )}
        {(spec.direction === "Upper" || spec.direction === "Range") && (
          <NumberInput
            label="≤"
            value={spec.max}
            disabled={!spec.enabled}
            onChange={(v) => onChange({ max: v })}
          />
        )}
        {spec.direction === "Lower" && (
          <NumberInput
            label="任意值 (无上限)"
            value="—"
            disabled
            onChange={() => {}}
          />
        )}
      </div>

      <details style={{ marginTop: 8 }}>
        <summary
          style={{
            cursor: "pointer",
            color: "var(--c-text-3)",
            fontSize: 10,
          }}
        >
          判定设置 · {enforcementLabel} · {acceptanceLabel}
        </summary>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 1fr",
            gap: 8,
            marginTop: 8,
            paddingTop: 8,
            borderTop: "1px dashed var(--c-border)",
          }}
        >
          <SelectInput
            label="约束级别"
            value={spec.enforcement}
            disabled={!spec.enabled}
            options={ENFORCEMENT_OPTIONS}
            onChange={(v) => onChange({ enforcement: v as Enforcement })}
          />
          <SelectInput
            label="判定方式"
            value={spec.acceptanceMode}
            disabled={!spec.enabled}
            options={ACCEPTANCE_OPTIONS}
            onChange={(v) =>
              onChange({ acceptanceMode: v as AcceptanceMode })
            }
          />
          <NumberInput
            label="判定小数位"
            value={spec.acceptanceMode === "Raw" ? "—" : spec.decimals}
            disabled={!spec.enabled || spec.acceptanceMode === "Raw"}
            min={0}
            max={6}
            step={1}
            onChange={(v) => onChange({ decimals: v })}
          />
          <NumberInput
            label="额外容差"
            value={spec.tolerance}
            disabled={!spec.enabled}
            min={0}
            step="any"
            onChange={(v) => onChange({ tolerance: v })}
          />
          <NumberInput
            label="安全余量（仅硬约束）"
            value={spec.margin}
            disabled={!spec.enabled || spec.enforcement !== "Hard"}
            min={0}
            step="any"
            onChange={(v) => onChange({ margin: v })}
          />
        </div>
        <div
          style={{
            marginTop: 6,
            color: "var(--c-text-3)",
            fontSize: 10,
          }}
        >
          {spec.enforcement === "Hard"
            ? "硬约束参与求解并决定方案可行性"
            : spec.enforcement === "Soft"
              ? "软约束允许输出，但会标记偏差"
              : "仅提示项不限制最低成本方案"}
        </div>
      </details>
    </div>
  );
}

function NumberInput({
  label,
  value,
  disabled,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  min?: number;
  max?: number;
  step?: number | "any";
  onChange: (v: string) => void;
}) {
  return (
    <label
      style={{
        background: "var(--c-bg)",
        borderRadius: 8,
        padding: "8px 10px",
        display: "flex",
        flexDirection: "column",
        gap: 2,
        opacity: disabled ? 0.5 : 1,
      }}
    >
      <span style={{ fontSize: 10, color: "var(--c-text-3)" }}>{label}</span>
      <input
        type="number"
        inputMode="decimal"
        value={value === "—" ? "" : value}
        disabled={disabled}
        min={min}
        max={max}
        step={step}
        placeholder={value === "—" ? "—" : "—"}
        onChange={(e) => onChange(e.target.value)}
        style={{
          background: "transparent",
          border: "none",
          outline: "none",
          fontSize: 14,
          fontWeight: 600,
          fontVariantNumeric: "tabular-nums",
          fontFamily: "inherit",
          color: "var(--c-text)",
          padding: 0,
        }}
      />
    </label>
  );
}

function SelectInput({
  label,
  value,
  disabled,
  options,
  onChange,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  options: ReadonlyArray<{ value: string; label: string }>;
  onChange: (value: string) => void;
}) {
  return (
    <label
      style={{
        background: "var(--c-bg)",
        borderRadius: 8,
        padding: "8px 10px",
        display: "flex",
        flexDirection: "column",
        gap: 2,
        opacity: disabled ? 0.5 : 1,
      }}
    >
      <span style={{ fontSize: 10, color: "var(--c-text-3)" }}>{label}</span>
      <select
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        style={{
          background: "transparent",
          border: "none",
          outline: "none",
          fontSize: 13,
          fontWeight: 600,
          fontFamily: "inherit",
          color: "var(--c-text)",
          padding: 0,
        }}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}
