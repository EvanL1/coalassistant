import type { PenaltyTemplate } from "./penalty";

// 只存 localStorage, 不进 UserStorageSnapshot ——
// 服务端用户存储是显式 PostgreSQL 列(coal_prefs/contract/quantity/user_coals),
// 新增顶层键需要建表迁移, 不在本期范围. 代价: 模板不跨设备同步.
const KEY_PENALTY_TEMPLATE = "doudou_blend.penalty_template.v1";

/**
 * 模板变更事件名。订阅方用这个常量, 不要手打字符串字面量 ——
 * 打错字不会报错, 只会让那个屏幕永远收不到刷新事件。
 */
export const PENALTY_TEMPLATE_EVENT = "doudou:penalty_template_changed";

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
  window.dispatchEvent(new CustomEvent(PENALTY_TEMPLATE_EVENT));
}

export function clearPenaltyTemplate(): void {
  localStorage.removeItem(KEY_PENALTY_TEMPLATE);
  window.dispatchEvent(new CustomEvent(PENALTY_TEMPLATE_EVENT));
}
