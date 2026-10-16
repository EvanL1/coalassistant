import type { BlendRequest, Coal } from "../types";

/** 形态的简称; 精煤是默认形态, 不标. 词表外的形态照原样显示. */
const FORM_SHORT: Record<string, string> = {
  精煤: "",
  筛选精煤: "筛精",
  浮选精煤: "浮精",
};

/** 煤的分类标签: 煤种, 非精煤时并列形态, 如 "肥煤·筛精". 都没写为空串. */
export function coalTag(coal: Pick<Coal, "coal_type" | "form">): string {
  const form = coal.form ? (FORM_SHORT[coal.form] ?? coal.form) : "";
  return [coal.coal_type ?? "", form].filter(Boolean).join("·");
}

/**
 * 求解请求里每种煤的分类标签 (煤种, 非精煤时并列形态): 进 LP 的煤和按估算价求解的煤
 * (price_factor.extra_coals) 都算. 今日屏按请求取, 不另查 master —— 用户新增的煤只在请求里有.
 */
export function coalTypeMap(request: BlendRequest): Map<string, string> {
  const types = new Map<string, string>();
  const coals = [...request.coals, ...(request.price_factor?.extra_coals ?? [])];
  for (const coal of coals) {
    const tag = coalTag(coal);
    if (tag) types.set(coal.name, tag);
  }
  return types;
}
