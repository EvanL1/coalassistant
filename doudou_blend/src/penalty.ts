import type { Direction, Penalty, PurchaseClause, PurchaseTerms } from "./types";

/** 模板里的一条条款: 只有条款本身, 保证值由每种煤各自提供. */
export interface PenaltyTemplateClause {
  indicator: string;
  direction: Direction;
  penalty: Penalty;
}

/** 全局扣款模板: 行业通用条款, 所有煤默认套用. */
export interface PenaltyTemplate {
  clauses: PenaltyTemplateClause[];
  contract_moisture?: number | null;
  moisture_excess_double_threshold?: number | null;
}

/** 单煤覆盖: 只列出与模板不同的条款. */
export interface CoalPenaltyOverride {
  clauses?: PenaltyTemplateClause[];
  contract_moisture?: number | null;
  moisture_excess_double_threshold?: number | null;
}

/**
 * 把合同原文的"每 step 个单位扣 amount 元"换算成 元/吨·单位.
 * 合同写"每超 0.1% 扣 8 元/吨" ⇒ tierRate(0.1, 8) = 80.
 * 让用户照抄合同, 不做心算 —— 8 与 80 差一个量级.
 */
export function tierRate(step: number, amount: number): number {
  if (!Number.isFinite(step) || step <= 0 || !Number.isFinite(amount)) return 0;
  return amount / step;
}

/**
 * 合并全局模板与单煤覆盖, 配上该煤的保证值, 产出 core 需要的 PurchaseTerms.
 * 没有保证值的指标不产出条款(没有保证值就无从判定偏离).
 * 返回 null 表示该煤无可用采购条款, 应省略 purchase_terms 字段.
 */
export function mergePurchaseTerms(
  template: PenaltyTemplate | null,
  override: CoalPenaltyOverride | undefined,
  guarantees: Record<string, number>,
): PurchaseTerms | null {
  const byIndicator = new Map<string, PenaltyTemplateClause>();
  for (const clause of template?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }
  for (const clause of override?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }

  const clauses: PurchaseClause[] = [];
  for (const [indicator, clause] of byIndicator) {
    const guarantee = guarantees[indicator];
    if (!Number.isFinite(guarantee)) continue;
    clauses.push({
      indicator,
      direction: clause.direction,
      guarantee,
      penalty: clause.penalty,
    });
  }

  // 没有任何条款落地 = 该煤未配置采购扣款, 即使模板带了合同水分也不该套用
  // (水分双倍计入是"该煤有采购合同"的推论, 不该在用户没给任何保证值时生效).
  if (clauses.length === 0) return null;

  const contract_moisture =
    override?.contract_moisture ?? template?.contract_moisture ?? null;
  const moisture_excess_double_threshold =
    override?.moisture_excess_double_threshold ?? template?.moisture_excess_double_threshold ?? null;

  return { clauses, contract_moisture, moisture_excess_double_threshold };
}
