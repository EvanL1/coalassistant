import { INDICATOR_LABEL } from "../types";
import type { BlendResult, InfeasibleBound } from "../types";

/**
 * 不可行诊断面板.
 *
 * 就地渲染在成功时放结果的位置 —— 页头、输入摘要、报价时效这些上下文照常留在屏上:
 * 求解失败时用户最需要看见的恰恰是自己喂进去的东西 (哪些煤在池里、合同卡得多死、
 * 买多少吨). 失败路径要比成功路径给得更多, 而不是更少.
 *
 * `infeasible_bounds` 每一项都是 core 算出来的真凶: 单独把它放宽到"最好能做到"
 * 那个数就能求出配方. 空数组不等于没问题, 而是没有哪一项单独放宽够用 —— 这时如实
 * 说明, 不硬凑一个指标让用户白跑一趟.
 */

/**
 * 执行界比合同界更紧时才需要解释 —— 那时这一行看着已经达标, 不说破就是工具在自相矛盾.
 * 判定规则放宽的那一侧 (截断把 ≤10 放成按 ≤10.0999 执行) 不必打扰用户.
 */
function tighterThanContract(bound: InfeasibleBound): boolean {
  const gap =
    bound.direction === "Upper"
      ? bound.required - bound.enforced
      : bound.enforced - bound.required;
  return gap > 0.005;
}

/**
 * 为什么比合同界紧, 只能看 margin, 不能看方向.
 * 截断判定在下限一侧同样会收紧 (合同 ≥14.95 按 ≥15.0 执行) 且一点余量都没设,
 * 那时印"含安全余量"就是给用户编了一个不存在的原因 —— 与"到厂价/实际成本"那次
 * 标签印错是同一类错.
 */
function tighteningCause(bound: InfeasibleBound): string {
  return bound.margin > 0 ? `含 ${bound.margin} 安全余量` : "按合同判定规则";
}

/** 化验单分辨率是 0.01, 诊断表按这个精度读就够 (与混合指标的 4 位展示无关). */
function formatBound(value: number): string {
  return value.toFixed(2);
}

const cellStyle = {
  padding: "4px 6px",
  borderTop: "1px solid var(--c-border, #e5e7eb)",
  textAlign: "right",
} as const;

export function InfeasiblePanel({ result }: { result: BlendResult }) {
  const bounds = result.infeasible_bounds ?? [];
  // ok 却没有成本结构 = 结果不完整 (畸形或存量记录), 不是合同不可行 —— 标题和
  // 说明都得跟着变, 否则界面会对着一个成功的求解喊"不可行".
  const incomplete = result.ok;
  const reason =
    result.reason ??
    (incomplete ? "求解成功但没有返回成本结构, 无法展示方案." : "求解未给出原因.");

  return (
    <div className="card" style={{ borderLeft: "4px solid var(--c-danger)" }}>
      <div className="card-title" style={{ color: "var(--c-danger)" }}>
        {incomplete ? "✗ 结果不完整" : "✗ 不可行"}
      </div>
      <p style={{ margin: "0 0 10px", fontSize: 13 }}>{reason}</p>

      {bounds.length > 0 && (
        <>
          <p style={{ margin: "0 0 8px", fontSize: 12, color: "var(--c-text-3)" }}>
            其余约束都成立时, 下面这些项达不到要求. 把其中任意一项改成「放宽到」那一列
            的数, 就能求出配方.
          </p>
          <table
            className="infeasible-table"
            style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}
          >
            <thead>
              <tr>
                <th style={{ ...cellStyle, textAlign: "left", borderTop: "none" }}>
                  指标
                </th>
                <th style={{ ...cellStyle, borderTop: "none" }}>合同要求</th>
                <th style={{ ...cellStyle, borderTop: "none" }}>最好能做到</th>
                <th style={{ ...cellStyle, borderTop: "none" }}>放宽到</th>
              </tr>
            </thead>
            <tbody>
              {bounds.map((bound) => {
                const sign = bound.direction === "Upper" ? "≤" : "≥";
                return (
                  <tr key={`${bound.indicator}-${bound.direction}`}>
                    <td style={{ ...cellStyle, textAlign: "left" }}>
                      {INDICATOR_LABEL[bound.indicator] ?? bound.label_zh}
                    </td>
                    <td style={cellStyle}>
                      <div>
                        {sign}
                        {formatBound(bound.required)}
                      </div>
                      {/* 收紧时必须说破: 否则这一行看着已经达标, 用户会以为
                          工具在胡说, 也不知道真正要让开的是哪条线. */}
                      {tighterThanContract(bound) && (
                        <div style={{ color: "var(--c-text-3)" }}>
                          {tighteningCause(bound)}, 按 {sign}
                          {formatBound(bound.enforced)} 执行
                        </div>
                      )}
                    </td>
                    <td style={{ ...cellStyle, color: "var(--c-danger)" }}>
                      {formatBound(bound.achievable)}
                    </td>
                    <td style={cellStyle}>
                      {sign}
                      {formatBound(bound.relax_to)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </>
      )}

      {bounds.length === 0 && !incomplete && (
        <p style={{ margin: 0, fontSize: 12, color: "var(--c-text-3)" }}>
          没有单独一项约束能解释这次不可行: 冲突牵涉两项以上, 或者煤池本身就不够.
          需要同时放宽多项, 或去「煤池」启用更多煤源.
        </p>
      )}
    </div>
  );
}
