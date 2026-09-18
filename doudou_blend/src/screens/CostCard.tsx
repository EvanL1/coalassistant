import { INDICATOR_LABEL } from "../types";
import type { CostBreakdown } from "../types";
import type { OrphanedGuaranteeWarning } from "../domain/resolvedCoal";

const YUAN = (value: number) => `${value.toFixed(2)} 元/吨`;

/** 拆成整数/小数两段, 配合 .cost-int/.cost-dec 两段式大字号. */
function formatPrice(n: number): { int: string; dec: string } {
  const [intPart, decPart] = n.toFixed(2).split(".");
  return { int: intPart, dec: decPart };
}

function indicatorLabel(indicator: string): string {
  return INDICATOR_LABEL[indicator] ?? indicator;
}

/**
 * 成本卡: 大字号主位 = 净成本(net_per_ton), 下方明细行 = 到厂价 / 买入修正 /
 * 预计扣款.
 *
 * 净成本才是真实吨成本 —— LP 就是按它求最优的, 到厂价启用计价条款后只是报价,
 * 所以大字号主位必须跟着 net_per_ton 走, 不能停在 cif_per_ton (那会把这个功能
 * 本该让用户看见的数字重新藏起来). 没有计价条款时 net_per_ton === cif_per_ton,
 * 大字号数值跟老界面完全一致, 标签(最低到厂价)也不变.
 *
 * 买入修正/预计扣款为 0 时隐藏该行.
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
  const netPerTon = cost.net_per_ton ?? cost.cif_per_ton;
  const { int: costInt, dec: costDec } = formatPrice(netPerTon);

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
      <div className="cost-label">最低到厂价</div>
      <div className="cost-amount">
        <span className="cost-int">{costInt}</span>
        <span className="cost-dec">.{costDec}</span>
        <span className="cost-unit">元/吨</span>
      </div>
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
    </div>
  );
}
