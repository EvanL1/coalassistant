/**
 * 采购扣款模板 (买入侧) 录入.
 *
 * 买入侧与卖出侧方向相反: 你买的煤没达到采购合同的保证值, 供应商扣你的钱 ——
 * 你少付, 到厂价其实更低. 条款是行业通用的, 所以录一次当模板套到所有煤上;
 * 每种煤自己的保证值在煤卡里填 (见 CoalEditor 的「采购保证值」).
 *
 * 只存本机 (penaltyStorage), 不跨设备同步 —— 换设备后煤卡里的保证值同步过来了
 * 而模板没有, 今日屏会把这种煤报成"缺条款", 不会悄悄按零扣款算.
 */
import { useState } from "react";
import { PenaltyEditor } from "./PenaltyEditor";
import {
  emptyPenaltyDraft,
  indicatorUnit,
  templateFromDraft,
  templateToDraft,
  type TemplateClauseDraft,
  type TemplateDraft,
} from "./penalty";
import {
  clearPenaltyTemplate,
  getPenaltyTemplate,
  setPenaltyTemplate,
} from "./penaltyStorage";
import { INDICATOR_LABEL, INDICATOR_ORDER } from "./types";

const DIRECTION_OPTIONS = [
  { value: "Upper", label: "越低越好（超了扣钱）" },
  { value: "Lower", label: "越高越好（不够扣钱）" },
] as const;

export function PenaltyTemplateEditor() {
  const [draft, setDraft] = useState<TemplateDraft>(() =>
    templateToDraft(getPenaltyTemplate()),
  );
  const [savedFlag, setSavedFlag] = useState(false);

  // 整份模板只换算校验这一次: 能不能保存、写盘的内容、每条条款的红字都读它.
  // `error` 是模板层面的问题, `clauseErrors[i]` 是第 i 条条款自己的问题,
  // 两者各显示各的, 不会互相遮住.
  const { template, error, clauseErrors } = templateFromDraft(draft);

  function patchClause(index: number, patch: Partial<TemplateClauseDraft>) {
    setDraft({
      ...draft,
      clauses: draft.clauses.map((clause, i) =>
        i === index ? { ...clause, ...patch } : clause,
      ),
    });
  }

  function addClause() {
    const used = new Set(draft.clauses.map((clause) => clause.indicator));
    const indicator = INDICATOR_ORDER.find((key) => !used.has(key)) ?? "A";
    setDraft({
      ...draft,
      clauses: [
        ...draft.clauses,
        { indicator, direction: "Upper", penalty: emptyPenaltyDraft() },
      ],
    });
  }

  function removeClause(index: number) {
    setDraft({
      ...draft,
      clauses: draft.clauses.filter((_, i) => i !== index),
    });
  }

  function save() {
    if (!template) return;
    setPenaltyTemplate(template);
    setSavedFlag(true);
    setTimeout(() => setSavedFlag(false), 2000);
  }

  function clear() {
    if (!confirm("清空采购扣款模板？\n各种煤已填的保证值会留着，但不再折算成买入扣款。")) {
      return;
    }
    clearPenaltyTemplate();
    setDraft(templateToDraft(null));
  }

  return (
    <div style={{ padding: "4px 2px" }}>
      <div style={{ fontSize: 11, color: "var(--c-text-3)", lineHeight: 1.6 }}>
        采购合同对你买进的煤也有保证值和扣款：这里录一次通用条款，
        每种煤自己的保证值在煤卡里填。买入扣款是供应商扣你的钱，你少付，到厂价更低。
      </div>

      {draft.clauses.map((clause, index) => (
        <div
          key={index}
          style={{
            marginTop: 10,
            padding: "10px 12px",
            background: "var(--c-card)",
            borderRadius: 10,
          }}
        >
          <div style={{ display: "flex", gap: 8, alignItems: "flex-end" }}>
            <label style={{ flex: 1, display: "flex", flexDirection: "column", gap: 2 }}>
              <span style={{ fontSize: 10, color: "var(--c-text-3)" }}>指标</span>
              <select
                aria-label={`第 ${index + 1} 条条款的指标`}
                value={clause.indicator}
                onChange={(event) => patchClause(index, { indicator: event.target.value })}
                style={selectStyle}
              >
                {INDICATOR_ORDER.map((key) => (
                  <option key={key} value={key}>
                    {INDICATOR_LABEL[key]}
                  </option>
                ))}
              </select>
            </label>
            <label style={{ flex: 2, display: "flex", flexDirection: "column", gap: 2 }}>
              <span style={{ fontSize: 10, color: "var(--c-text-3)" }}>这项的方向</span>
              <select
                aria-label={`第 ${index + 1} 条条款的方向`}
                value={clause.direction}
                onChange={(event) =>
                  patchClause(index, {
                    direction: event.target.value === "Lower" ? "Lower" : "Upper",
                  })
                }
                style={selectStyle}
              >
                {DIRECTION_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              onClick={() => removeClause(index)}
              style={{
                color: "var(--c-danger)",
                fontSize: 11,
                background: "none",
                paddingBottom: 6,
              }}
            >
              删掉本条
            </button>
          </div>

          <PenaltyEditor
            draft={clause.penalty}
            unit={indicatorUnit(clause.indicator)}
            direction={clause.direction}
            error={clauseErrors[index]}
            onChange={(penalty) => patchClause(index, { penalty })}
          />
        </div>
      ))}

      <button
        type="button"
        onClick={addClause}
        style={{
          marginTop: 10,
          fontSize: 12,
          color: "var(--c-primary)",
          background: "none",
          padding: 0,
        }}
      >
        + 加一条条款
      </button>

      <div style={{ marginTop: 12, display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
        <MoistureInput
          label="合同水分（%）"
          hint="按扣量：湿煤只按折算后的吨数付货款"
          value={draft.contractMoisture}
          onChange={(contractMoisture) => setDraft({ ...draft, contractMoisture })}
        />
        <MoistureInput
          label="水分双倍阈值（%）"
          hint="超过这个水分，超出部分按 2 倍计"
          value={draft.doubleThreshold}
          onChange={(doubleThreshold) => setDraft({ ...draft, doubleThreshold })}
        />
      </div>

      {error && (
        <div style={{ marginTop: 10, fontSize: 11, color: "var(--c-danger)", lineHeight: 1.5 }}>
          {error}
        </div>
      )}

      <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
        <button
          className="btn btn-secondary"
          style={{ flex: 1, color: "var(--c-danger)" }}
          onClick={clear}
        >
          清空模板
        </button>
        <button
          className="btn btn-primary"
          style={{ flex: 1, opacity: template ? 1 : 0.5 }}
          disabled={!template}
          onClick={save}
        >
          {savedFlag ? "✓ 已保存" : "保存模板"}
        </button>
      </div>
    </div>
  );
}

const selectStyle = {
  background: "var(--c-bg)",
  borderRadius: 6,
  border: "1px solid var(--c-border)",
  outline: "none",
  fontSize: 12,
  fontWeight: 600,
  fontFamily: "inherit",
  color: "var(--c-text)",
  padding: "6px",
} as const;

function MoistureInput({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint: string;
  value: string;
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
      }}
    >
      <span style={{ fontSize: 10, color: "var(--c-text-3)" }}>{label}</span>
      <input
        type="number"
        inputMode="decimal"
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder="不设"
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
      <span style={{ fontSize: 9, color: "var(--c-text-3)" }}>{hint}</span>
    </label>
  );
}
