/**
 * 后台数据层: 配煤参数 / 煤阶 k 校准 / CSR 回归模型 / 煤库覆盖层.
 *
 * 全部走同源 Cookie 会话, 服务端按管理员角色鉴权 (非管理员 403).
 * API 密钥的增删查在 apiKeys.ts, 这里不重复.
 */

export interface AdminSettings {
  /** 煤阶交互罚项系数; null 或 0 = 不启用. 设置后服务端对所有求解自动生效. */
  rank_interaction_k: number | null;
}

export interface CalibrationPoint {
  id: string;
  occurred_at: string;
  base_csr: number;
  measured_csr: number;
  rank_variance: number;
}

export interface Calibration {
  n: number;
  skipped: number;
  k: number | null;
  intercept: number | null;
  k_std_error: number | null;
  d_min: number | null;
  d_max: number | null;
  recommended: boolean;
  reason: string;
  points: CalibrationPoint[];
}

export interface CsrModelStatus {
  samples: number;
  required: number;
  ready: boolean;
  r_squared: number | null;
}

/** 与服务端 master_data.rs 的 FIELD 白名单一致. */
export const OVERRIDE_FIELDS = ["S", "A", "V", "G", "Y", "petro", "CSR", "M"] as const;
export type OverrideField = (typeof OVERRIDE_FIELDS)[number];

export const CONFIDENCE_LEVELS = ["high", "medium", "low"] as const;
export type OverrideConfidence = (typeof CONFIDENCE_LEVELS)[number];

export interface CoalOverride {
  coal_name: string;
  field: string;
  value: number;
  source: string;
  confidence: string;
  updated_at: string;
  updated_by?: string | null;
}

export interface CoalUpdate {
  coal: string;
  field: OverrideField;
  value: number;
  source: string;
  confidence: OverrideConfidence;
}

/**
 * 错误体有两种: 普通错误 `{reason}`, 批量校验失败 `{errors: [...]}` (422).
 * 都原样回显给用户 —— "这个字段基线已有" 之类的理由比 HTTP 码有用得多.
 */
async function request(path: string, init?: RequestInit): Promise<unknown> {
  const response = await fetch(`/api/${path}`, {
    credentials: "same-origin",
    ...init,
  });
  const text = await response.text();
  if (!response.ok) {
    let reason = text;
    try {
      const body = JSON.parse(text) as { reason?: string; errors?: string[] };
      reason = body.reason ?? body.errors?.join("; ") ?? text;
    } catch {
      // 非 JSON 错误体, 原样回显
    }
    throw new Error(reason || `HTTP ${response.status}`);
  }
  return text ? JSON.parse(text) : null;
}

const JSON_HEADERS = { "Content-Type": "application/json" };

export async function getSettings(): Promise<AdminSettings> {
  const result = (await request("admin/settings", { cache: "no-store" })) as {
    settings: AdminSettings;
  };
  return result.settings;
}

export async function setRankInteractionK(k: number | null): Promise<AdminSettings> {
  const result = (await request("admin/settings", {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify({ rank_interaction_k: k }),
  })) as { settings: AdminSettings };
  return result.settings;
}

export async function getCalibration(): Promise<Calibration> {
  const result = (await request("admin/calibration", { cache: "no-store" })) as {
    calibration: Calibration;
  };
  return result.calibration;
}

export async function getCsrModel(): Promise<CsrModelStatus> {
  const result = (await request("admin/csr-model", { cache: "no-store" })) as {
    model: CsrModelStatus;
  };
  return result.model;
}

export async function listOverrides(): Promise<CoalOverride[]> {
  const result = (await request("master/overrides", { cache: "no-store" })) as {
    overrides?: CoalOverride[];
  };
  return result.overrides ?? [];
}

export async function addOverrides(updates: CoalUpdate[]): Promise<void> {
  await request("master/coals", {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ updates }),
  });
}

export async function deleteOverride(coal: string, field: string): Promise<void> {
  await request(
    `master/coals/${encodeURIComponent(coal)}/${encodeURIComponent(field)}`,
    { method: "DELETE" },
  );
}
