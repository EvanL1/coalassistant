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
import type { CoalMaster, Spec } from "../types";

interface FormSpec {
  indicator: string;
  direction: "Upper" | "Lower" | "Range";
  min: string;
  max: string;
  enabled: boolean;
}

function specToForm(s: Spec): FormSpec {
  return {
    indicator: s.indicator,
    direction: s.direction,
    min: s.min != null ? String(s.min) : "",
    max: s.max != null ? String(s.max) : "",
    enabled: s.enabled !== false,
  };
}

function formToSpec(f: FormSpec): Spec {
  const min = f.min.trim() === "" ? null : parseFloat(f.min);
  const max = f.max.trim() === "" ? null : parseFloat(f.max);
  return {
    indicator: f.indicator,
    direction: f.direction,
    min: Number.isFinite(min) ? min : null,
    max: Number.isFinite(max) ? max : null,
    enabled: f.enabled,
  };
}

interface Props {
  onBack?: () => void;
}

export function ContractScreen({ onBack }: Props) {
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
          {onBack && (
            <button
              onClick={onBack}
              style={{
                fontSize: 13,
                color: "var(--c-text-3)",
                marginBottom: 4,
                padding: 0,
              }}
            >
              ‹ 返回我的
            </button>
          )}
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
        提示: 改完合同后回到「今日」tab, 点重新计算就会用新合同求解
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
    </div>
  );
}

function NumberInput({
  label,
  value,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  disabled?: boolean;
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
