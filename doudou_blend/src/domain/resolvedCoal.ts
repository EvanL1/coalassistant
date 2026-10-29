import type { CoalPref, CoalPrefs } from "../storage";
import type {
  Coal,
  CoalStatus,
  MasterCoalEntry,
  PurchaseTerms,
} from "../types";
import { INDICATOR_ORDER } from "../types";
import { normalizeCoalName } from "./coalName";

export type CoalOrigin = "master" | "user";

/**
 * 单煤真实报价拿不到, 煤价一律按质量估算 (价格因子). 不读录入价, 也不读 master 里的
 * fob/frt —— 真实报价是在估算之后去谈的, 不进求解.
 */
export type CoalReadiness =
  | "ready"
  | "disabled"
  | "hidden"
  | "duplicate_name"
  | "invalid_override";

const INDICATOR_KEYS = new Set<string>(INDICATOR_ORDER);

function isRecord(value: unknown): value is Record<string, unknown> {
  return (
    value != null &&
    typeof value === "object" &&
    !Array.isArray(value)
  );
}

export interface ResolvedCoal {
  origin: CoalOrigin;
  base: MasterCoalEntry;
  name: string;
  region: string | null;
  coal_type: string | null;
  /** 产品形态; master 未写为 null (schema 约定默认精煤). */
  form: string | null;
  status: CoalStatus;
  props: Partial<Record<string, number>>;
  hidden: boolean;
  requestedEnabled: boolean;
  effectiveEnabled: boolean;
  hasOverrides: boolean;
  overriddenProps: Partial<Record<string, true>>;
  /** 独立保留数据有效性, 不能被停用、隐藏或重名状态遮蔽. */
  invalidOverride: boolean;
  readiness: CoalReadiness;
}

function isValidNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

export function resolveCoal(
  base: MasterCoalEntry,
  pref: CoalPref | null,
  origin: CoalOrigin,
): ResolvedCoal {
  const rawPref: unknown = pref;
  const safePref = isRecord(rawPref) ? (rawPref as CoalPref) : null;
  let invalidOverride = rawPref != null && safePref == null;
  invalidOverride ||= safePref?.invalid_data === true;

  let requestedEnabled = base.status === "verified";
  // 已停用 (archived) 是煤矿本身的状态, 一票否决: 用户偏好里的启用拉不回来.
  if (base.status === "archived") {
    requestedEnabled = false;
  } else if (safePref?.enabled != null) {
    if (typeof safePref.enabled === "boolean") {
      requestedEnabled = safePref.enabled;
    } else {
      requestedEnabled = false;
      invalidOverride = true;
    }
  }

  let hidden = false;
  if (safePref?.hidden != null) {
    if (typeof safePref.hidden === "boolean") {
      hidden = safePref.hidden;
    } else {
      invalidOverride = true;
    }
  }

  const props: Partial<Record<string, number>> = {};
  const baseProps: unknown = base.props;
  if (!isRecord(baseProps)) {
    invalidOverride = true;
  } else {
    for (const [key, value] of Object.entries(baseProps)) {
      if (!INDICATOR_KEYS.has(key)) {
        invalidOverride = true;
        continue;
      }
      if (value == null) continue;
      if (isValidNumber(value)) {
        props[key] = value;
      } else {
        invalidOverride = true;
      }
    }
  }

  const overriddenProps: Partial<Record<string, true>> = {};
  const propsOverride = safePref?.props_override as unknown;
  if (propsOverride != null) {
    if (
      typeof propsOverride !== "object" ||
      Array.isArray(propsOverride)
    ) {
      invalidOverride = true;
    } else {
      for (const [key, value] of Object.entries(propsOverride)) {
        if (!INDICATOR_KEYS.has(key)) {
          invalidOverride = true;
          continue;
        }
        if (value == null) continue;
        if (!isValidNumber(value)) {
          invalidOverride = true;
          continue;
        }
        props[key] = value;
        if (value !== base.props[key]) {
          overriddenProps[key] = true;
        }
      }
    }
  }

  const effectiveEnabled = requestedEnabled && !hidden;
  let readiness: CoalReadiness;
  if (hidden) {
    readiness = "hidden";
  } else if (!requestedEnabled) {
    readiness = "disabled";
  } else if (invalidOverride) {
    readiness = "invalid_override";
  } else {
    readiness = "ready";
  }

  const hasOverrides = Object.keys(overriddenProps).length > 0;

  return {
    origin,
    base,
    name: base.name,
    region: base.region ?? null,
    coal_type: base.coal_type ?? null,
    form: base.form ?? null,
    status: base.status,
    props,
    hidden,
    requestedEnabled,
    effectiveEnabled,
    hasOverrides,
    overriddenProps,
    invalidOverride,
    readiness,
  };
}

export function resolveCoalPool(
  masterCoals: MasterCoalEntry[],
  userCoals: MasterCoalEntry[],
  prefs: CoalPrefs,
): ResolvedCoal[] {
  const resolved = [
    ...userCoals.map((coal) =>
      resolveCoal(coal, prefs[coal.name] ?? null, "user"),
    ),
    ...masterCoals.map((coal) =>
      resolveCoal(coal, prefs[coal.name] ?? null, "master"),
    ),
  ];
  const nameCounts = new Map<string, number>();
  for (const coal of resolved) {
    const normalized = normalizeCoalName(coal.name);
    nameCounts.set(normalized, (nameCounts.get(normalized) ?? 0) + 1);
  }
  return resolved.map((coal) =>
    (nameCounts.get(normalizeCoalName(coal.name)) ?? 0) > 1
      ? {
          ...coal,
          effectiveEnabled: false,
          readiness: "duplicate_name",
        }
      : coal,
  );
}

/**
 * 求解用的煤: 一律不带报价 (fob/frt 记 0), 放进价格因子 extra_coals, 由核心按估算价
 * 并入煤池 (estimate_unpriced). 只有启用且数据有效的煤参与.
 *
 * `purchaseTerms` 由调用方传入: 单次 `resolvePurchaseTerms` 扫描 (domain/purchaseTerms.ts)
 * 的结果, 不要在这里重新调用 `mergePurchaseTerms`. 核心并入煤池时条款随煤走, 保证值扣款
 * 照算在估算价上. `null`/缺省 = 不产出这个字段 (用 `undefined`, 让 JSON.stringify 省略它).
 */
export function toEstimateCoal(
  coal: ResolvedCoal,
  purchaseTerms?: PurchaseTerms | null,
): Coal | null {
  if (coal.readiness !== "ready") return null;
  return {
    name: coal.name,
    props: { ...coal.props },
    fob: 0,
    frt: 0,
    purchase_terms: purchaseTerms ?? undefined,
    coal_type: coal.coal_type ?? undefined,
    form: coal.form ?? undefined,
    region: coal.region ?? undefined,
  };
}
