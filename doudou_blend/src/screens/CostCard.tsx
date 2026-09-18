import { INDICATOR_LABEL } from "../types";
import type { CostBreakdown } from "../types";
import type { OrphanedGuaranteeWarning } from "../domain/purchaseTerms";
import { hasCostAdjustments } from "../domain/costBreakdown";

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
 * 成本卡: 大字号主位 = 实际成本(net_per_ton), 下方明细行 = 到厂价 / 买入修正 /
 * 预计扣款.
 *
 * net_per_ton 才是真实吨成本 —— LP 就是按它求最优的, 到厂价启用计价条款后只是
 * 报价, 所以大字号主位必须跟着 net_per_ton 走, 不能停在 cif_per_ton (那会把这个
 * 功能本该让用户看见的数字重新藏起来).
 *
 * 标签跟着数值变, 不能写死"最低到厂价": 没有计价条款时 net_per_ton ===
 * cif_per_ton, 大字号数值和标签都跟老界面完全一致; 一旦两者不等, 标签必须换成
 * "最低实际成本"。标签写死不变的话, 这个大字号会在数值已经不是到厂价之后继续
 * 顶着"到厂价"的名字 —— 用户是透过标签去读这个数字的, 标签比数字本身更容易被
 * 无条件相信, 印错标签比印错数字更危险, 恰好是这整个功能想消灭的那类"报价当
 * 真实成本"的静默误导。
 *
 * 不用"最低净成本": "净"暗示"扣掉之后更便宜", 但卖出扣款会把这个数字推到
 * 到厂价**之上**(不是更便宜, 是更贵), 恰恰是最需要这行标签说清楚的那种情况 ——
 * "净"字在这时是误导, "实际成本"只陈述事实, 不暗示方向。`net_per_ton` 这个
 * 字段名不改, 只改中文界面文案。
 *
 * 三行明细 (到厂价/买入修正/预计扣款) 都在"没内容"时隐藏: 买入修正/预计扣款
 * 为 0 时各自隐藏; 到厂价则是当它跟大字号数值相等时隐藏(此时再列一行纯属
 * 重复), 也就是 `!detailed` 时明细行整体不渲染 —— 没配置计价条款的用户看到的
 * 就是过去那一个大数字, 一行明细都不多.
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
  // 用共享判定, 不要在这里重新拼 hasAdjust||hasPenalty ——
  // TodayScreen.tsx 的 buildOrderText 曾经用 |net-cif|>eps 单独判过一次,
  // 两笔调整刚好抵消时会算出不同结果, 见 domain/costBreakdown.ts。
  const detailed = hasCostAdjustments(cost);
  const netPerTon = cost.net_per_ton ?? cost.cif_per_ton;
  const { int: costInt, dec: costDec } = formatPrice(netPerTon);

  return (
    <div className="cost-card">
      {orphanedGuarantees.length > 0 && (
        <div className="cost-orphan-warning" role="alert">
          <div className="cost-orphan-warning-title">
            ⚠ 有煤的采购扣款没算进成本
          </div>
          <div className="cost-orphan-warning-body">
            以下煤配了采购保证值, 但找不到匹配的扣款条款, 暂按无扣款计入成本
            —— 请核对并录入这些煤对应的扣款条款:
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
      <div className="cost-label">{detailed ? "最低实际成本" : "最低到厂价"}</div>
      <div className="cost-amount" data-testid="cost-headline">
        <span className="cost-int">{costInt}</span>
        <span className="cost-dec">.{costDec}</span>
        <span className="cost-unit">元/吨</span>
      </div>
      {/* 到厂价明细行只在实际成本偏离报价时才有意义; 没有计价条款时两者相等,
          再列一行纯属重复大字号, 索性不渲染 —— 界面跟没有这个功能时完全一样. */}
      {detailed && (
        <div className="cost-row">
          <span>到厂价</span>
          <span data-testid="cost-cif">{YUAN(cost.cif_per_ton)}</span>
        </div>
      )}
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
