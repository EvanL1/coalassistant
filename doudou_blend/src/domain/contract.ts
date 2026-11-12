import type { Spec } from "../types";

/**
 * 合同里启用的项是否填完整: 上限型要有上限, 下限型要有下限, 区间型两端都要且下限不超过上限.
 * 与核心 quality.rs 的合同校验同一套规则; 今日页据此判断"不可行是不是合同没填完",
 * 不去匹配核心报错的中文原文 (文案一改, 指引就悄悄退回"放宽约束/启用更多煤").
 */
export function isContractIncomplete(
  specs: readonly Pick<Spec, "direction" | "min" | "max" | "enabled">[],
): boolean {
  return specs.some((spec) => {
    if (spec.enabled === false) return false;
    const hasMin = typeof spec.min === "number" && Number.isFinite(spec.min);
    const hasMax = typeof spec.max === "number" && Number.isFinite(spec.max);
    if (spec.direction === "Upper") return !hasMax;
    if (spec.direction === "Lower") return !hasMin;
    return !hasMin || !hasMax || (spec.min as number) > (spec.max as number);
  });
}

/**
 * 发给核心前按方向丢掉用不到的那一端: 上限型的残留下限、下限型的残留上限核心求解时不看,
 * 但校验会报"下限大于上限", 而合同页上那一端根本不显示, 用户找不到要改哪里.
 */
export function withoutUnusedBound(spec: Spec): Spec {
  if (spec.direction === "Upper" && spec.min != null) return { ...spec, min: null };
  if (spec.direction === "Lower" && spec.max != null) return { ...spec, max: null };
  return spec;
}
