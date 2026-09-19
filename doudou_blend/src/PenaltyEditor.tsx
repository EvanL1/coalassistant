/**
 * 计价条款录入 (档位表 + 拒收线), 合同屏(卖出侧)与煤池屏采购扣款模板(买入侧)共用.
 *
 * 两栏录入是这个组件存在的理由: 合同写"每超 0.1% 扣 8 元/吨", core 要的却是
 * 80 元/吨·%. 让用户照抄合同原文的两个数, 换算交给 `penaltyFromDraft` ——
 * 填成 8 不会有任何东西看起来是坏的, 但每吨扣款差一个量级.
 *
 * 组件自己不判对错: `error` 由父层调一次 `penaltyFromDraft` 得出, 同一个结果
 * 既用来显示错误也用来决定能不能存, 不会出现"红字在这、保存却过了"。
 */
import { deviationNoun } from "./penalty";
import type { PenaltyDraft, TierDraft } from "./penalty";

interface Props {
  draft: PenaltyDraft;
  /** 该指标的计量单位: "%" 或 "点". 用 `indicatorUnit()` 取, 别手写. */
  unit: string;
  /** 上限型超上限拒收, 下限型低于下限拒收 —— 文案必须跟着方向走. */
  direction: "Upper" | "Lower";
  /** `penaltyFromDraft` 的报错; null = 这条条款填对了. */
  error: string | null;
  onChange: (draft: PenaltyDraft) => void;
}

export function PenaltyEditor({ draft, unit, direction, error, onChange }: Props) {
  // 上限型是"超出", 下限型是"不足" —— 用词跟着方向走, 与报错文案同一处定义.
  const noun = deviationNoun(direction);

  function patchTier(index: number, patch: Partial<TierDraft>) {
    onChange({
      ...draft,
      tiers: draft.tiers.map((tier, i) =>
        i === index ? { ...tier, ...patch } : tier,
      ),
    });
  }

  function addTier() {
    // 沿用上一档的"每 ___": 同一份合同的档位通常是同一个粒度.
    const previous = draft.tiers[draft.tiers.length - 1];
    onChange({
      ...draft,
      tiers: [...draft.tiers, { step: previous?.step ?? "0.1", amount: "", width: "" }],
    });
  }

  function removeTier(index: number) {
    const tiers = draft.tiers.filter((_, i) => i !== index);
    // 删掉末档后, 新末档的覆盖宽度必须一并清掉: 末档吃剩下的全部超出, 留着
    // 旧宽度会让这条条款一直报"末档不能填本档覆盖", 而用户看不到那一栏.
    if (tiers.length > 0) {
      tiers[tiers.length - 1] = { ...tiers[tiers.length - 1], width: "" };
    }
    onChange({ ...draft, tiers });
  }

  return (
    <div style={{ marginTop: 8 }}>
      {draft.tiers.map((tier, index) => (
        <div
          key={index}
          style={{
            display: "flex",
            alignItems: "center",
            flexWrap: "wrap",
            gap: 4,
            fontSize: 12,
            color: "var(--c-text-2)",
            marginBottom: 6,
          }}
        >
          <span>第 {index + 1} 档 · 每</span>
          <InlineNumber
            ariaLabel={`第 ${index + 1} 档 每多少${unit}`}
            value={tier.step}
            onChange={(step) => patchTier(index, { step })}
          />
          <span>{unit} 扣</span>
          <InlineNumber
            ariaLabel={`第 ${index + 1} 档扣款（元/吨）`}
            value={tier.amount}
            onChange={(amount) => patchTier(index, { amount })}
          />
          <span>元/吨</span>
          {index < draft.tiers.length - 1 && (
            <>
              <span>· 本档覆盖</span>
              <InlineNumber
                ariaLabel={`第 ${index + 1} 档本档覆盖（${unit}）`}
                value={tier.width}
                onChange={(width) => patchTier(index, { width })}
              />
              <span>{unit}</span>
            </>
          )}
          {draft.tiers.length > 1 && (
            <button
              type="button"
              aria-label={`删掉第 ${index + 1} 档`}
              onClick={() => removeTier(index)}
              style={{
                marginLeft: "auto",
                color: "var(--c-danger)",
                fontSize: 11,
                background: "none",
              }}
            >
              删掉本档
            </button>
          )}
        </div>
      ))}

      <button
        type="button"
        onClick={addTier}
        style={{
          fontSize: 11,
          color: "var(--c-primary)",
          background: "none",
          padding: 0,
          marginBottom: 6,
        }}
      >
        + 加一档（{noun}更多、扣得更狠）
      </button>

      <div style={{ fontSize: 10, color: "var(--c-text-3)", marginBottom: 6 }}>
        {noun}量按档位从低到高依次计费: 第 1 档先吃掉最小的那一段{noun}, 填满才进下一档,
        末档吃掉剩下的全部。
      </div>

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
        <span style={{ fontSize: 10, color: "var(--c-text-3)" }}>
          拒收线（{direction === "Upper" ? "超过" : "低于"}它整批不收，不再按扣款算）
        </span>
        <input
          type="number"
          inputMode="decimal"
          aria-label={`拒收线（${direction === "Upper" ? "超过" : "低于"}它整批不收）`}
          value={draft.reject}
          onChange={(event) => onChange({ ...draft, reject: event.target.value })}
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

      {error && (
        <div
          style={{
            marginTop: 6,
            fontSize: 11,
            color: "var(--c-danger)",
            lineHeight: 1.5,
          }}
        >
          {error}
        </div>
      )}
    </div>
  );
}

function InlineNumber({
  ariaLabel,
  value,
  onChange,
}: {
  ariaLabel: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <input
      type="number"
      inputMode="decimal"
      aria-label={ariaLabel}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      style={{
        width: 64,
        background: "var(--c-bg)",
        borderRadius: 6,
        border: "1px solid var(--c-border)",
        outline: "none",
        fontSize: 13,
        fontWeight: 600,
        fontVariantNumeric: "tabular-nums",
        fontFamily: "inherit",
        color: "var(--c-text)",
        padding: "4px 6px",
      }}
    />
  );
}
