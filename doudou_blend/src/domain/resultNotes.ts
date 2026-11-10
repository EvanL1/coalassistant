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

/** 核心返回的文字照原样存, 只在显示时把 "LP" 这类术语换成人话. */
export function friendlySolverText(text: string): string {
  if (text.trim() === "约束冲突, LP 不可行") {
    return "合同要求互相冲突, 现有的煤怎么配都达不到";
  }
  return text.replace(/LP\s*不可行/g, "配不出满足合同的方案");
}
