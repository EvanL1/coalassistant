/** 跟 blend_kit_rs schema 对齐的 TypeScript 类型. */

export type IndicatorKey = "S" | "A" | "V" | "G" | "Y" | "petro" | "CSR" | "M";
export type AcceptanceMode = "Raw" | "Truncate" | "Round";
export type Enforcement = "Hard" | "Soft" | "Advisory";
export type EvaluationMethod =
  | "Linear"
  | "ProvisionalLinear"
  | "AffineCalibration"
  | "Histogram"
  | "Moments"
  | "Regression"
  | "Unavailable";
export type EvaluationStatus =
  | "Pass"
  | "TolerancePass"
  | "Unverified"
  | "Fail";
export type QualityStatus = "Verified" | "Estimated" | "NeedsReview";
export type ModelKind = "GAffine" | "CsrRidge";

export const INDICATOR_LABEL: Record<string, string> = {
  S: "硫",
  A: "灰",
  V: "挥发",
  G: "粘结",
  Y: "胶质",
  petro: "岩相",
  CSR: "焦炭强度",
  M: "水分",
};

export const INDICATOR_ORDER: IndicatorKey[] = [
  "S",
  "A",
  "V",
  "G",
  "Y",
  "petro",
  "CSR",
  "M",
];

export type Direction = "Upper" | "Lower" | "Range";

export interface AcceptanceRule {
  mode: AcceptanceMode;
  decimals?: number | null;
  tolerance: number;
}

export interface Spec {
  indicator: string;
  direction: Direction;
  min?: number | null;
  max?: number | null;
  enabled?: boolean;
  /** 安全余量: LP 内部上限收紧/下限抬高 margin, 展示仍用合同原界限. */
  margin?: number | null;
  acceptance?: AcceptanceRule | null;
  enforcement?: Enforcement;
}

/** 单煤煤岩数据 (MT/T 507 化验单: 反射率直方图 + 镜质组含量). */
export interface Petrography {
  /** [bin 中值 R(%), 频率] 列表, 频率可未归一化. */
  hist?: [number, number][];
  /** 镜质组体积含量 (%), 混合权重修正用. */
  vitrinite_pct: number;
  /** 没有直方图时可提供平均反射率. */
  mean?: number | null;
  /** 没有直方图时可提供反射率标准差. */
  std_dev?: number | null;
}

export interface Coal {
  name: string;
  props: Partial<Record<string, number>>;
  fob: number;
  frt: number;
  /** 可选煤岩数据; 提供时混煤 σ 走直方图精确计算. */
  petrography?: Petrography | null;
}

/** 本次指标评估实际采用的模型摘要. */
export interface ModelSummary {
  version: string;
  kind: ModelKind;
  sample_count: number;
  cv_mae: number;
  p90_abs_error: number;
  bias: number;
  in_domain: boolean;
}

export interface BlendRequest {
  coals: Coal[];
  specs: Spec[];
  total_quantity?: number | null;
  truncate_decimal?: boolean;
}

export interface CostBreakdown {
  fob_per_ton: number;
  frt_per_ton: number;
  cif_per_ton: number;
  total_fob?: number | null;
  total_frt?: number | null;
  total_cif?: number | null;
}

export interface OrderItem {
  coal: string;
  ratio: number;
  tons?: number | null;
  fob_amount?: number | null;
  frt_amount?: number | null;
  cif_amount?: number | null;
}

export interface IndicatorCheck {
  indicator: string;
  label_zh: string;
  value: number;
  min?: number | null;
  max?: number | null;
  slack?: number | null;
  binding: boolean;
  proxy_value?: number | null;
  evaluated_value?: number | null;
  judged_value?: number | null;
  uncertainty?: number | null;
  /** 旧历史结果可能缺失；界面按线性/传统 slack 兼容显示. */
  method?: EvaluationMethod;
  status?: EvaluationStatus;
  model?: ModelSummary | null;
}

/** 岩相凹口检测结果. */
export interface Notch {
  r: number;
  depth_ratio: number;
}

/** 岩相精确校验 (直方图合成 + 全方差定律), 与线性代理值不同. */
export interface PetrographyCheck {
  mean: number;
  sigma: number;
  sigma_max?: number | null;
  sigma_ok?: boolean | null;
  notch?: Notch | null;
  refine_iterations: number;
}

export interface BlendResult {
  ok: boolean;
  reason?: string | null;
  recipe: Record<string, number>;
  cost?: CostBreakdown | null;
  orders: OrderItem[];
  indicator_check: IndicatorCheck[];
  /** 参配煤缺煤岩数据时无此字段. */
  petrography_check?: PetrographyCheck | null;
  warnings: string[];
  /** 以下字段由混合质量引擎写入；旧历史结果可能缺失. */
  quality_status?: QualityStatus;
  evaluation_iterations?: number;
}

/** 混合后 6 项指标 (CSR 回归自变量 X). */
export interface MixedIndicators {
  s: number;
  a: number;
  v: number;
  g: number;
  y: number;
  m: number;
}

/** 回填的混煤实测化验值. 缺省/null = 本次不更新该项. */
export interface MeasuredQuality {
  s?: number | null;
  a?: number | null;
  v?: number | null;
  g?: number | null;
  y?: number | null;
  m?: number | null;
  csr?: number | null;
}

/** 历史方案 (跨后端统一形状). mixed/实测各列支撑「回填实测焦质」数据闭环. */
export interface HistoryRecord {
  id: string;
  occurred_at: string;
  contract_name: string;
  cost_cif: number;
  recipe: Record<string, number>;
  /** 混合后 6 项指标 (回归 X); null = 旧记录无此数据, 不提供回填入口. */
  mixed: MixedIndicators | null;
  /** 回填的实测 CSR (回归 y); null = 未回填. */
  csr_measured: number | null;
  /** 混煤实测化验回填 (信任对照 + G 修正模型样本); null = 未回填. */
  s_measured: number | null;
  a_measured: number | null;
  v_measured: number | null;
  g_measured: number | null;
  y_measured: number | null;
  m_measured: number | null;
}

// ===== Master schema =====

export type CoalStatus = "verified" | "active" | "draft" | "incomplete" | "archived";
export type Confidence = "high" | "medium" | "low";

export interface MasterCoalEntry {
  name: string;
  region?: string | null;
  coal_type?: string | null;
  status: CoalStatus;
  props: Partial<Record<string, number>>;
  fob?: number | null;
  frt?: number | null;
  confidence?: Record<string, Confidence>;
  note?: string | null;
}

export interface DefaultContract {
  name: string;
  specs: Spec[];
}

export interface CoalMaster {
  version: string;
  updated_at: string;
  description: string;
  default_contract: DefaultContract;
  coals: MasterCoalEntry[];
}
