/** 跟 blend_kit_rs schema 对齐的 TypeScript 类型. */

export type IndicatorKey = "S" | "A" | "V" | "G" | "Y" | "petro" | "CSR" | "M";
export type AcceptanceMode = "Raw" | "Truncate" | "Round";
export type Enforcement = "Hard" | "Soft" | "Advisory" | "Priced";
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

/** 扣款档位: 覆盖 width 宽度的偏离, 按 rate (元/吨·指标单位) 计费. */
export interface PenaltyTier {
  /** 本档覆盖的偏离宽度; 省略 = 末档, 无上限. */
  width?: number | null;
  /** 元/吨 per 1 个指标单位. 合同"每 0.1% 扣 8 元" → 80. */
  rate: number;
}

/** 单项指标的计价条款. tiers 的 rate 必须严格递增(凸性). */
export interface Penalty {
  tiers: PenaltyTier[];
  /** 拒收线: 越过即硬不可行. */
  reject: number;
}

/** 单条采购合同计价条款 (买入侧). */
export interface PurchaseClause {
  indicator: string;
  /** 只接受 Upper / Lower. */
  direction: Direction;
  guarantee: number;
  penalty: Penalty;
}

/** 采购合同条款; 全局模板与单煤覆盖已在前端合并完毕. */
export interface PurchaseTerms {
  clauses: PurchaseClause[];
  /** 合同水分 (%). */
  contract_moisture?: number | null;
  /** 超过该水分 (%) 时, 超出部分按 2 倍计入有效水分 M_eff. */
  moisture_excess_double_threshold?: number | null;
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
  penalty?: Penalty | null;
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
  purchase_terms?: PurchaseTerms | null;
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
  /** 到厂价. 扣款条款落地后仅作展示用, 实际优化目标是 net_per_ton. */
  cif_per_ton: number;
  total_fob?: number | null;
  total_frt?: number | null;
  total_cif?: number | null;
  /**
   * 买入侧扣款折扣 + 水分折算带来的到厂价修正合计, 元/吨. 负值 = 成本下降.
   * 可选: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
   */
  purchase_adjust_per_ton?: number;
  /**
   * 卖出侧质量扣款合计, 元/吨.
   * 可选: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
   */
  penalty_per_ton?: number;
  /**
   * 真实吨成本 = cif + purchase_adjust + penalty.
   * 可选: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
   */
  net_per_ton?: number;
  total_purchase_adjust?: number | null;
  total_penalty?: number | null;
  total_net?: number | null;
}

export interface OrderItem {
  coal: string;
  ratio: number;
  tons?: number | null;
  fob_amount?: number | null;
  frt_amount?: number | null;
  cif_amount?: number | null;
  /**
   * 该煤买入侧修正后的单价 (元/吨), 采购按此价核对.
   * 可选: 兼容扣款条款上线前存量 OrderItem 记录 (无此字段).
   */
  cif_eff_per_ton?: number;
  /**
   * 该煤买入侧修正后的订单金额 (元) = cif_eff_per_ton × tons.
   * 与 cif_amount 的差别是含买入扣款与水分折算; 结账金额看这项.
   * 可选: 兼容扣款条款上线前存量 OrderItem 记录 (无此字段).
   */
  cif_eff_amount?: number | null;
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
  /** 本项卖出侧扣款, 元/吨. 非计价指标为 None. */
  penalty_per_ton?: number | null;
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
