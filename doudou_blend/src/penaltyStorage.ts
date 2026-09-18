import type { PenaltyTemplate } from "./penalty";

// 只存 localStorage, 不进 UserStorageSnapshot ——
// 服务端用户存储是显式 PostgreSQL 列(coal_prefs/contract/quantity/user_coals),
// 新增顶层键需要建表迁移, 不在本期范围. 代价: 模板不跨设备同步.
const KEY_PENALTY_TEMPLATE = "doudou_blend.penalty_template.v1";

export function getPenaltyTemplate(): PenaltyTemplate | null {
  try {
    const raw = localStorage.getItem(KEY_PENALTY_TEMPLATE);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PenaltyTemplate;
    if (!parsed || !Array.isArray(parsed.clauses)) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function setPenaltyTemplate(template: PenaltyTemplate): void {
  localStorage.setItem(KEY_PENALTY_TEMPLATE, JSON.stringify(template));
  window.dispatchEvent(new Event("doudou:penalty_template_changed"));
}

export function clearPenaltyTemplate(): void {
  localStorage.removeItem(KEY_PENALTY_TEMPLATE);
  window.dispatchEvent(new Event("doudou:penalty_template_changed"));
}
