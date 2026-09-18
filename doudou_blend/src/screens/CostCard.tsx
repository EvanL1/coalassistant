import { INDICATOR_LABEL } from "../types";
import type { CostBreakdown } from "../types";
import type { OrphanedGuaranteeWarning } from "../domain/resolvedCoal";

const YUAN = (value: number) => `${value.toFixed(2)} 元/吨`;

function indicatorLabel(indicator: string): string {
  return INDICATOR_LABEL[indicator] ?? indicator;
}

/**
 * 成本卡: 到厂价 / 买入修正 / 预计扣款 / 净成本.
 * 净成本才是真实吨成本 —— LP 就是按它求最优的.
 * 修正与扣款为 0 时隐藏该行, 保持无计价条款用户的界面不变.
 *
 * `orphanedGuarantees` (Step 8): 有采购保证值却匹配不到扣款条款的煤 ——
 * 通常是全局扣款模板没同步到这台设备 (模板只存 localStorage, 见
 * penaltyStorage.ts), 这些煤本该被计价却被静默按无扣款处理, 后果是花钱算错,
 * 所以做成显眼的告警而不是角落里的小字.
 */
export function CostCard({
  cost,
  orphanedGuarantees = [],
}: {
  cost: CostBreakdown;
  orphanedGuarantees?: OrphanedGuaranteeWarning[];
}) {
  const hasAdjust = Math.abs(cost.purchase_adjust_per_ton ?? 0) > 1e-6;
  const hasPenalty = Math.abs(cost.penalty_per_ton ?? 0) > 1e-6;
  const detailed = hasAdjust || hasPenalty;
  const netPerTon = cost.net_per_ton ?? cost.cif_per_ton;

  return (
    <div className="cost-card">
      {orphanedGuarantees.length > 0 && (
        <div className="cost-orphan-warning" role="alert">
          <div className="cost-orphan-warning-title">
            ⚠ 采购扣款模板在本设备缺失
          </div>
          <div className="cost-orphan-warning-body">
            以下煤配了采购保证值, 但找不到匹配的扣款条款, 已按无扣款计入成本
            —— 通常是采购扣款模板没同步到这台设备, 请在这台设备上重新录入模板:
          </div>
          <ul className="cost-orphan-warning-list">
            {orphanedGuarantees.map((w) => (
              <li key={w.coal}>
                {w.coal}: {w.indicators.map(indicatorLabel).join("、")}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="cost-row">
        <span>到厂价</span>
        <span data-testid="cost-cif">{YUAN(cost.cif_per_ton)}</span>
      </div>
      {hasAdjust && (
        <div className="cost-row cost-row--adjust">
          <span>买入修正</span>
          <span data-testid="cost-adjust">
            {YUAN(cost.purchase_adjust_per_ton ?? 0)}
          </span>
        </div>
      )}
      {hasPenalty && (
        <div className="cost-row cost-row--penalty">
          <span>预计扣款</span>
          <span data-testid="cost-penalty">
            {YUAN(cost.penalty_per_ton ?? 0)}
          </span>
        </div>
      )}
      <div className={detailed ? "cost-row cost-row--net" : "cost-row"}>
        <span>{detailed ? "净成本" : "合计"}</span>
        <span data-testid="cost-net">{YUAN(netPerTon)}</span>
      </div>
    </div>
  );
}
