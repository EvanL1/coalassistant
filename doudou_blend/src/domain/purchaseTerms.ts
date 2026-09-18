import type { CoalPrefs } from "../storage";
import type { PurchaseTerms } from "../types";
import { mergePurchaseTerms, type PenaltyTemplate } from "../penalty";
import type { ResolvedCoal } from "./resolvedCoal";

/** 采购扣款模板缺失告警 (Step 8): 一种煤 + 它匹配不到条款的那些指标. */
export interface OrphanedGuaranteeWarning {
  coal: string;
  indicators: string[];
}

/** 单种煤跑完 `mergePurchaseTerms` 后的结果: 求解用的条款 + 孤儿指标. */
export interface CoalPurchaseTerms {
  coal: string;
  /** 挂到对应 `Coal.purchase_terms` 上; null = 该煤没有可用条款(不产出字段). */
  terms: PurchaseTerms | null;
  /** 有保证值却匹配不到条款的指标; 空数组 = 没有孤儿. */
  orphanedIndicators: string[];
}

/**
 * 对参与求解的煤各跑一次 `mergePurchaseTerms` —— 只跑一次, 而不是求解请求的
 * `purchase_terms` 和孤儿保证值告警(CostCard)各自对同一批输入重算一遍。
 *
 * 两个调用点各自重算同一件事, 是这个项目撞过很多次的缺陷形状: 数据或口径一旦
 * 改动, 只改了其中一处, 两处结果就悄悄分叉。这里把"给一种煤配出它的采购条款"
 * 只做一次, 两个下游(`purchaseTermsByCoalName` / `extractOrphanedGuarantees`)
 * 都从同一份扫描结果派生, 保证不可能分叉。
 *
 * 只处理 `readiness === "ready"` 的煤 —— 这跟 `toBlendCoal`(resolvedCoal.ts)
 * 实际会塞进求解请求的煤集合完全一致: `readiness !== "ready"` 的煤
 * `toBlendCoal` 直接返回 null, 根本不会出现在请求里。早先孤儿检测是在
 * `effectiveEnabled` 上扫描的, 会把还没配好价格、这次求解压根用不上的煤也
 * 纳入告警, 让用户为不影响这次成本的煤操心。
 */
export function resolvePurchaseTerms(
  pool: readonly ResolvedCoal[],
  prefs: CoalPrefs,
  template: PenaltyTemplate | null,
): CoalPurchaseTerms[] {
  const result: CoalPurchaseTerms[] = [];
  for (const coal of pool) {
    if (coal.readiness !== "ready") continue;
    const pref = prefs[coal.name];
    const guarantees = pref?.purchase_guarantees;
    if (!guarantees || Object.keys(guarantees).length === 0) {
      result.push({ coal: coal.name, terms: null, orphanedIndicators: [] });
      continue;
    }
    const { terms, orphanedGuarantees } = mergePurchaseTerms(
      template,
      pref?.purchase_override,
      guarantees,
    );
    result.push({
      coal: coal.name,
      terms,
      orphanedIndicators: orphanedGuarantees,
    });
  }
  return result;
}

/** 从单次扫描结果派生 CostCard 用的孤儿告警. */
export function extractOrphanedGuarantees(
  entries: readonly CoalPurchaseTerms[],
): OrphanedGuaranteeWarning[] {
  return entries
    .filter((entry) => entry.orphanedIndicators.length > 0)
    .map((entry) => ({
      coal: entry.coal,
      indicators: entry.orphanedIndicators,
    }));
}

/** 从单次扫描结果派生"煤名 → 采购条款"查找表, 供拼 `Coal.purchase_terms` 用. */
export function purchaseTermsByCoalName(
  entries: readonly CoalPurchaseTerms[],
): Map<string, PurchaseTerms | null> {
  return new Map(entries.map((entry) => [entry.coal, entry.terms]));
}
