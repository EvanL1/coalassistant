import type { BlendRequest } from "../types";

/**
 * 求解请求里每种煤的煤种: 进 LP 的煤和按估算价求解的煤 (price_factor.extra_coals) 都算.
 * 今日屏按请求取, 不另查 master —— 用户新增的煤只在请求里有煤种.
 */
export function coalTypeMap(request: BlendRequest): Map<string, string> {
  const types = new Map<string, string>();
  const coals = [...request.coals, ...(request.price_factor?.extra_coals ?? [])];
  for (const coal of coals) {
    if (coal.coal_type) types.set(coal.name, coal.coal_type);
  }
  return types;
}
