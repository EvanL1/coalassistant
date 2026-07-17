import type { CoalPref, CoalPrefs } from "../storage";
import type {
  Coal,
  CoalStatus,
  MasterCoalEntry,
} from "../types";
import { INDICATOR_ORDER } from "../types";
import { normalizeCoalName } from "./coalName";

export type CoalOrigin = "master" | "user";

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
  fob: number | null;
  frt: number | null;
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

  let cif =
    fob.value != null && frt.value != null
      ? fob.value + frt.value
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

export function toBlendCoal(coal: ResolvedCoal): Coal | null {
  if (
    coal.readiness !== "ready" ||
    coal.fob == null ||
    coal.frt == null
  ) {
    return null;
  }
  return {
    name: coal.name,
    props: { ...coal.props },
    fob: coal.fob,
    frt: coal.frt,
  };
}
