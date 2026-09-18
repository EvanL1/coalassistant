import type { CoalPref, CoalPrefs } from "../storage";
import type {
  Coal,
  CoalStatus,
  MasterCoalEntry,
} from "../types";
import { INDICATOR_ORDER } from "../types";
import { mergePurchaseTerms, type PenaltyTemplate } from "../penalty";
import { normalizeCoalName } from "./coalName";
import { driftedFob, type PriceAnchor } from "./priceDrift";

export type CoalOrigin = "master" | "user";

/** 价格漂移上下文; 不传 = 不推算, 一切按录入价(旧行为). */
export interface PriceDriftContext {
  /** 锚点煤构成的自建价格指数 */
  anchor: PriceAnchor;
  /** master 的 updated_at, 给从没单独录过价的煤兜底当录价日 */
  masterUpdatedAt?: string | null;
}

export type CoalReadiness =
  | "ready"
  | "disabled"
  | "hidden"
  | "missing_fob"
  | "missing_frt"
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
  status: CoalStatus;
  props: Partial<Record<string, number>>;
  /** 用户录入的原始出厂价 */
  fob: number | null;
  /** 这个 fob 是哪天的口径 */
  fob_quoted_at: string | null;
  /** 按锚点推算的当前出厂价; null = 无法推算, 求解退回 fob */
  fob_drifted: number | null;
  /** 推算用的涨跌比例; null = 未推算 */
  drift_ratio: number | null;
  frt: number | null;
  /** 到厂价 = (推算价 ?? 录入价) + 运费 */
  cif: number | null;
  hidden: boolean;
  requestedEnabled: boolean;
  effectiveEnabled: boolean;
  hasOverrides: boolean;
  overriddenProps: Partial<Record<string, true>>;
  readiness: CoalReadiness;
}

function isValidNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function resolvePrice(
  base: number | null | undefined,
  override: unknown,
): { value: number | null; overridden: boolean; invalid: boolean } {
  if (override == null) {
    return {
      value: isValidNumber(base) ? base : null,
      overridden: false,
      invalid: false,
    };
  }
  if (!isValidNumber(override)) {
    return { value: null, overridden: false, invalid: true };
  }
  return {
    value: override,
    overridden: override !== base,
    invalid: false,
  };
}

export function resolveCoal(
  base: MasterCoalEntry,
  pref: CoalPref | null,
  origin: CoalOrigin,
  drift?: PriceDriftContext | null,
): ResolvedCoal {
  const rawPref: unknown = pref;
  const safePref = isRecord(rawPref) ? (rawPref as CoalPref) : null;
  let invalidOverride = rawPref != null && safePref == null;
  invalidOverride ||= safePref?.invalid_data === true;

  let requestedEnabled = base.status === "verified";
  if (safePref?.enabled != null) {
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

  const fob = resolvePrice(base.fob, safePref?.fob_override);
  const frt = resolvePrice(base.frt, safePref?.frt_override);
  invalidOverride ||= fob.invalid || frt.invalid;

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

  // 录价日: 显式记录 > 改价时间戳 > master 口径. 定不出来就不推算.
  const explicitQuotedAt =
    typeof safePref?.fob_quoted_at === "string" ? safePref.fob_quoted_at : null;
  const legacyQuotedAt =
    safePref?.fob_override != null && typeof safePref.updated_at === "string"
      ? safePref.updated_at.slice(0, 10)
      : null;
  const fobQuotedAt =
    explicitQuotedAt ?? legacyQuotedAt ?? drift?.masterUpdatedAt ?? null;

  const fobDrifted =
    drift?.anchor != null && fob.value != null
      ? driftedFob(fob.value, fobQuotedAt, drift.anchor)
      : null;
  const driftRatioValue =
    fobDrifted != null && fob.value != null && fob.value > 0
      ? fobDrifted / fob.value
      : null;

  const effectiveFob = fobDrifted ?? fob.value;
  let cif =
    effectiveFob != null && frt.value != null
      ? effectiveFob + frt.value
      : null;
  if (cif != null && !Number.isFinite(cif)) {
    cif = null;
    invalidOverride = true;
  }

  const effectiveEnabled = requestedEnabled && !hidden;
  let readiness: CoalReadiness;
  if (hidden) {
    readiness = "hidden";
  } else if (!requestedEnabled) {
    readiness = "disabled";
  } else if (invalidOverride) {
    readiness = "invalid_override";
  } else if (fob.value == null) {
    readiness = "missing_fob";
  } else if (frt.value == null) {
    readiness = "missing_frt";
  } else {
    readiness = "ready";
  }

  const hasOverrides =
    fob.overridden ||
    frt.overridden ||
    Object.keys(overriddenProps).length > 0;

  return {
    origin,
    base,
    name: base.name,
    region: base.region ?? null,
    coal_type: base.coal_type ?? null,
    status: base.status,
    props,
    fob: fob.value,
    fob_quoted_at: fobQuotedAt,
    fob_drifted: fobDrifted,
    drift_ratio: driftRatioValue,
    frt: frt.value,
    cif,
    hidden,
    requestedEnabled,
    effectiveEnabled,
    hasOverrides,
    overriddenProps,
    readiness,
  };
}

export function resolveCoalPool(
  masterCoals: MasterCoalEntry[],
  userCoals: MasterCoalEntry[],
  prefs: CoalPrefs,
  drift?: PriceDriftContext | null,
): ResolvedCoal[] {
  const resolved = [
    ...userCoals.map((coal) =>
      resolveCoal(coal, prefs[coal.name] ?? null, "user", drift),
    ),
    ...masterCoals.map((coal) =>
      resolveCoal(coal, prefs[coal.name] ?? null, "master", drift),
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

export function toBlendCoal(coal: ResolvedCoal): Coal | null {
  if (
    coal.readiness !== "ready" ||
    coal.fob == null ||
    coal.frt == null
  ) {
    return null;
  }
  // 求解用推算价: 拿几十天前的旧报价求解会系统性低估到厂成本.
  return {
    name: coal.name,
    props: { ...coal.props },
    fob: coal.fob_drifted ?? coal.fob,
    frt: coal.frt,
  };
}

/** 本次求解所用价格的可信度摘要. 无论是否发生推算都会返回. */
export interface PriceStatus {
  /** 参与求解的煤里最旧的报价日; null = 无法判定 */
  oldestQuotedAt: string | null;
  /** 用了推算价的煤数量; 0 = 全部按原始录入价 */
  driftedCount: number;
  /** 被推算煤的平均涨跌比例; null = 没有发生推算 */
  avgRatio: number | null;
  /** 参与推算的锚点煤 */
  anchors: string[];
}

/**
 * 汇总本次求解的价格可信度.
 *
 * 注意这里**总是**返回对象, 而不是"没推算就返回 null" —— 最需要向用户交代的
 * 恰恰是没有推算的默认态: 那时卡片上印着两位小数的到厂价和总额, 底下却是几十天
 * 前的报价. 把提示做成"功能生效后才显示"等于把警告装在了不需要警告的一侧.
 */
export function summarizePriceStatus(
  pool: readonly ResolvedCoal[],
  anchors: readonly string[],
): PriceStatus {
  const participating = pool.filter((coal) => coal.readiness === "ready");
  let oldestQuotedAt: string | null = null;
  for (const coal of participating) {
    const quoted = coal.fob_quoted_at;
    if (quoted && (oldestQuotedAt == null || quoted < oldestQuotedAt)) {
      oldestQuotedAt = quoted;
    }
  }
  const drifted = participating.filter(
    (coal) =>
      coal.drift_ratio != null && Math.abs(coal.drift_ratio - 1) > 1e-9,
  );
  const avgRatio =
    drifted.length > 0
      ? drifted.reduce((sum, coal) => sum + (coal.drift_ratio ?? 1), 0) /
        drifted.length
      : null;
  return {
    oldestQuotedAt,
    driftedCount: drifted.length,
    avgRatio,
    anchors: [...anchors],
  };
}

/** 采购扣款模板缺失告警 (Step 8): 一种煤 + 它匹配不到条款的那些指标. */
export interface OrphanedGuaranteeWarning {
  coal: string;
  indicators: string[];
}

/**
 * 扫描启用的煤, 找出"有采购保证值却在模板+覆盖里都找不到条款"的煤。
 *
 * 全局扣款模板只存 localStorage, 不跟 coal_prefs 一起同步到服务端
 * (见 penaltyStorage.ts); 换设备登录后每种煤的 purchase_guarantees 正常
 * 同步过来了, 但模板没有 —— 这些煤本该计价却被 mergePurchaseTerms 静默
 * 按无扣款处理, 两台设备算出的成本不一致但界面上什么都不会报错。
 *
 * 只看 `effectiveEnabled` 的煤: 用户已经停用/隐藏的煤即使配置不完整也不是
 * 当前会影响到的钱, 不该吓用户。
 */
export function collectOrphanedGuarantees(
  pool: readonly ResolvedCoal[],
  prefs: CoalPrefs,
  template: PenaltyTemplate | null,
): OrphanedGuaranteeWarning[] {
  const warnings: OrphanedGuaranteeWarning[] = [];
  for (const coal of pool) {
    if (!coal.effectiveEnabled) continue;
    const pref = prefs[coal.name];
    const guarantees = pref?.purchase_guarantees;
    if (!guarantees || Object.keys(guarantees).length === 0) continue;
    const { orphanedGuarantees } = mergePurchaseTerms(
      template,
      pref?.purchase_override,
      guarantees,
    );
    if (orphanedGuarantees.length > 0) {
      warnings.push({ coal: coal.name, indicators: orphanedGuarantees });
    }
  }
  return warnings;
}
