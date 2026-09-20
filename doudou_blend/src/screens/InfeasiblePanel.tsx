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

/** 差多少: 上限超出多少, 下限还差多少. */
function shortfall(bound: InfeasibleBound): number {
  return bound.direction === "Upper"
    ? bound.achievable - bound.required
    : bound.required - bound.achievable;
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
  return (
    <div className="card" style={{ borderLeft: "4px solid var(--c-danger)" }}>
      <div className="card-title" style={{ color: "var(--c-danger)" }}>
        ✗ 不可行
      </div>
      <p style={{ margin: "0 0 10px", fontSize: 13 }}>{result.reason}</p>

      {bounds.length > 0 ? (
        <>
          <p style={{ margin: "0 0 8px", fontSize: 12, color: "var(--c-text-3)" }}>
            其余约束都成立时, 下面这些项达不到合同要求. 单独放宽其中任意一项就能求出配方.
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
                <th style={{ ...cellStyle, borderTop: "none" }}>差多少</th>
              </tr>
            </thead>
            <tbody>
              {bounds.map((bound) => {
                const gap = shortfall(bound);
                return (
                  <tr key={`${bound.indicator}-${bound.direction}`}>
                    <td style={{ ...cellStyle, textAlign: "left" }}>
                      {INDICATOR_LABEL[bound.indicator] ?? bound.label_zh}
                    </td>
                    <td style={cellStyle}>
                      {bound.direction === "Upper" ? "≤" : "≥"}
                      {formatBound(bound.required)}
                    </td>
                    <td style={{ ...cellStyle, color: "var(--c-danger)" }}>
                      {formatBound(bound.achievable)}
                    </td>
                    {/* 合同界本身够得到、只是被安全余量或判定规则收紧时, 差值会 ≤0,
                        此时写死一个数字反而误导, 留空更老实. */}
                    <td style={cellStyle}>{gap > 0 ? formatBound(gap) : "—"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </>
      ) : (
        <p style={{ margin: 0, fontSize: 12, color: "var(--c-text-3)" }}>
          没有单独一项约束能解释这次不可行: 冲突牵涉两项以上, 或者煤池本身就不够.
          需要同时放宽多项, 或去「煤池」启用更多煤源.
        </p>
      )}
    </div>
  );
}
