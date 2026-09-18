import type { CostBreakdown } from "../types";

const EPSILON = 1e-6;

/**
 * 净成本是否偏离到厂价报价 —— 买入修正或卖出扣款任一非零就算.
 *
 * 唯一判定入口, 不要在各处各自用 `|net_per_ton - cif_per_ton| > eps` 重新推导:
 * 买入折扣与卖出扣款刚好互相抵消时 net === cif, 但两笔调整确实各自发生了,
 * 用户理应看到这两笔明细, 而不是被"合计没差异"糊弄成什么都没发生过。
 */
export function hasCostAdjustments(cost: CostBreakdown): boolean {
  return (
    Math.abs(cost.purchase_adjust_per_ton ?? 0) > EPSILON ||
    Math.abs(cost.penalty_per_ton ?? 0) > EPSILON
  );
}
