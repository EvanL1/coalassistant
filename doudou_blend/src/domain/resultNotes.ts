import type { BlendRequest, BlendResult } from "../types";

/**
 * 请求带了一铲吨数、却有煤没给铲数: 核心凑不成整铲, 退回了未凑整的配方.
 * 今日页解出来时不显示一般性提示, 这一条例外 —— 现场按铲配, 不说破就会照着非整铲的吨数去装.
 */
export function shovelRoundingMissed(
  request: Pick<BlendRequest, "shovel_tons">,
  result: Pick<BlendResult, "ok" | "orders">,
): boolean {
  return (
    request.shovel_tons != null &&
    result.ok &&
    result.orders.length > 0 &&
    result.orders.some((order) => order.shovels == null)
  );
}

export function shovelRoundingNotice(shovelTons: number): string {
  return `这次没凑成整铲 (一铲 ${shovelTons} 吨), 吨数没按整铲取整, 装车前请自行凑整`;
}

/**
 * 配方顶在合同界上时直接凑整会超标, 核心会留一点余量重新配再凑整.
 * 这样配出来比不凑整时贵一点, 要让用户知道贵在哪.
 */
export function shovelExtraCostNotice(extraPerTon: number): string {
  return `为凑成整铲留了余量重新配, 比不按整铲配贵 ${extraPerTon.toFixed(2)} 元/吨`;
}

/**
 * 没出配方属于哪一类. 今日页的指引与诊断面板都按它分支, 不各判一遍 (判两处迟早分叉).
 *  - incomplete: 求解成功却没有成本结构 (畸形或存量记录), 与合同无关;
 *  - contract: 合同本身没填完整, 放宽约束、加煤、重算都没用, 只能去合同页补全;
 *  - undetermined: 求解器没算出结论、也没能证实不可行, 合同也许本来配得出, 不能引导用户去放宽;
 *  - infeasible: 合同与煤池确实冲突.
 */
export type FailureKind = "incomplete" | "contract" | "undetermined" | "infeasible";

/**
 * contractIncomplete 由调用方拿发给核心的那份 specs 校验 (domain/contract.ts 的
 * isContractIncomplete), 不匹配核心报错的中文句子.
 */
export function failureKind(
  result: Pick<BlendResult, "ok" | "undetermined">,
  contractIncomplete: boolean,
): FailureKind {
  if (result.ok) return "incomplete";
  if (contractIncomplete) return "contract";
  return result.undetermined === true ? "undetermined" : "infeasible";
}

/** 核心返回的文字照原样存, 只在显示时把 "LP" 这类术语换成人话. */
export function friendlySolverText(text: string): string {
  if (text.trim() === "约束冲突, LP 不可行") {
    return "合同要求互相冲突, 现有的煤怎么配都达不到";
  }
  return text.replace(/LP\s*不可行/g, "配不出满足合同的方案");
}
