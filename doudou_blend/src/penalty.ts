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
  /**
   * 该煤明确不适用的模板条款指标: 即使有保证值也不产出条款, 且不计入孤儿保证值
   * (这是用户主动排除, 不是数据丢失). 真实采购合同确实会只对部分指标计价.
   */
  excluded_indicators?: string[];
  contract_moisture?: number | null;
  moisture_excess_double_threshold?: number | null;
}

/**
 * 合并结果. `terms` 供 core 使用; `orphanedGuarantees` 标出"有保证值却找不到
 * 条款"的指标 —— 调用方应据此提示用户, 而不是静默按无扣款处理.
 *
 * 孤儿保证值最常见的成因: 全局模板只存 localStorage 不跨设备同步
 * (见 penaltyStorage.ts), 换设备登录后 CoalPref 里的 purchase_guarantees
 * 随 coal_prefs 正常同步过来了, 但模板没有 —— 这种煤本该计价却悄悄零扣款,
 * 两台设备算出的成本会不一致但界面上什么都不会报错。
 */
export interface MergedPurchaseTerms {
  terms: PurchaseTerms | null;
  orphanedGuarantees: string[];
}

/**
 * 把合同原文的"每 step 个单位扣 amount 元"换算成 元/吨·单位.
 * 合同写"每超 0.1% 扣 8 元/吨" ⇒ tierRate({ step: 0.1, amount: 8 }) = 80.
 * 让用户照抄合同, 不做心算 —— 8 与 80 差一个量级。
 *
 * 用具名对象参数, 不用位置参数: `tierRate(8, 0.1)` 这种参数顺序传反的调用,
 * 位置参数会静默算出一个看似合理但错误的数字, 对象参数会直接类型对不上/
 * 值域检查失败, 编译期或运行期都能截住。
 *
 * 非法输入(step/amount 非有限数、step<=0、amount<0)返回 null, 不是 0 ——
 * 0 本身是合法的零费率档位(某档不扣钱), 不能拿它当错误信号, 调用方必须
 * 显式处理 null。
 */
export function tierRate({
  step,
  amount,
}: {
  step: number;
  amount: number;
}): number | null {
  if (!Number.isFinite(step) || step <= 0) return null;
  if (!Number.isFinite(amount) || amount < 0) return null;
  return amount / step;
}

/**
 * 合并全局模板与单煤覆盖, 配上该煤的保证值, 产出 core 需要的 PurchaseTerms。
 *
 * 规则:
 *   - 覆盖按指标逐条替换模板条款(不是整体替换); `excluded_indicators` 里的
 *     指标即使在模板或覆盖里有条款也不产出(实现"单煤覆盖压制模板条款",
 *     而不仅是新增/替换)。
 *   - 没有保证值的指标不产出条款(没有保证值就无从判定偏离), 也不算孤儿
 *     (这只是"没配置", 不是"配置丢了")。
 *   - 有保证值却在模板+覆盖里都找不到条款的指标(且未被显式排除), 计入
 *     `orphanedGuarantees` —— 调用方应提示用户, 而不是当作用户没配置。
 *   - `contract_moisture` / `moisture_excess_double_threshold`: 覆盖对象里
 *     显式写了这个键(哪怕值是 null)就用覆盖的值, 包括用 null 关掉该项;
 *     完全没写这个键才回退模板。用 `in` 判断键是否存在, 不用 `??`,
 *     因为 `??` 分不清"显式设为 null"和"压根没设置"。
 *   - `terms === null` 表示该煤没有任何可用采购条款, 调用方应省略
 *     `purchase_terms` 字段。
 */
export function mergePurchaseTerms(
  template: PenaltyTemplate | null,
  override: CoalPenaltyOverride | undefined,
  guarantees: Partial<Record<string, number>>,
): MergedPurchaseTerms {
  const byIndicator = new Map<string, PenaltyTemplateClause>();
  for (const clause of template?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }
  for (const clause of override?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }
  const excluded = new Set(override?.excluded_indicators ?? []);

  const clauses: PurchaseClause[] = [];
  const orphanedGuarantees: string[] = [];
  for (const [indicator, guarantee] of Object.entries(guarantees)) {
    if (typeof guarantee !== "number" || !Number.isFinite(guarantee)) continue;
    if (excluded.has(indicator)) continue;
    const clause = byIndicator.get(indicator);
    if (!clause) {
      orphanedGuarantees.push(indicator);
      continue;
    }
    clauses.push({
      indicator,
      direction: clause.direction,
      guarantee,
      penalty: clause.penalty,
    });
  }

  // 没有任何条款落地 = 该煤未配置采购扣款, 即使模板/覆盖带了合同水分也不该
  // 套用(水分双倍计入是"该煤有采购合同"的推论, 不该在没有条款时生效)。
  if (clauses.length === 0) {
    return { terms: null, orphanedGuarantees };
  }

  const contract_moisture =
    override != null && "contract_moisture" in override
      ? (override.contract_moisture ?? null)
      : (template?.contract_moisture ?? null);
  const moisture_excess_double_threshold =
    override != null && "moisture_excess_double_threshold" in override
      ? (override.moisture_excess_double_threshold ?? null)
      : (template?.moisture_excess_double_threshold ?? null);

  return {
    terms: { clauses, contract_moisture, moisture_excess_double_threshold },
    orphanedGuarantees,
  };
}
