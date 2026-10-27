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
  /** 煤种 (焦煤/肥煤/1/3焦煤/瘦煤…). 不进 LP; 用于展示、估价对标与按煤种取估价误差. */
  coal_type?: string | null;
  /** 产品形态 (精煤/筛选精煤/浮选精煤…), 与煤种并列. 不进 LP; 估价对标用. */
  form?: string | null;
  /** 产地 (如 "山西临汾"), 与煤种并列. 不进 LP; 估价对标用. */
  region?: string | null;
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
  /** 验算模式: 煤名 → 份数 (按总和归一). 给了就不求最优, 直接按此配比出结果. */
  fixed_ratios?: Record<string, number> | null;
  /** 煤阶交互罚项 (配煤不相容). 不传或 k = 0 = 不启用. */
  rank_interaction?: RankInteraction | null;
  /** 价格因子 (按质量估应值价). 默认只评估; estimate_unpriced 时无报价的煤按估算价进 LP. */
  price_factor?: PriceFactor | null;
  /** 价格稳健性分析 (煤价估不准时配方靠不靠得住). 不传 = 不分析. */
  price_robustness?: PriceRobustness | null;
  /** 一铲多少吨. 与 total_quantity 同给时每种煤要么不用、要么至少一铲, 且按整铲配. */
  shovel_tons?: number | null;
  /** 单煤配比下限: 煤名 → 占比 (0~1). 给了的煤一定入选且不低于此比例. */
  min_ratios?: Record<string, number>;
}

/** 价格稳健性分析配置; 字段缺省用核心默认值 (容差 10, 搜索范围 300 元/吨). */
export interface PriceRobustness {
  /** 成本相差多少元/吨以内算"差不多". */
  tolerance?: number;
  /** 翻转门槛的搜索范围 (元/吨). */
  swing_range?: number;
}

/** CSR配 = Σxᵢ·CSRᵢ − k·D, D = 配比加权的煤阶 (Ro) 方差. k ≥ 0, 须用本厂实测标定. */
export interface RankInteraction {
  k: number;
}

/** 单条升贴水规则. 费率: 点 每 1 个指标单位 (标准品 = 1000 点). direction 只用 Upper/Lower. */
export interface PriceAdjust {
  indicator: string;
  direction: Direction;
  base: number;
  premium_rate: number;
  discount_rate: number;
  clamp?: [number, number] | null;
  linear_range?: [number, number] | null;
  enabled?: boolean;
}

export interface PriceFactor {
  adjusts: PriceAdjust[];
  /** 市场公开价: 标准品 (1000 点) 对应的元/吨; 缺省 = 由请求煤池反推. */
  market_price?: number | null;
  /** 只评估、不进 LP 的煤 (如没录报价的煤): 照样打分给估算价, 不参与配方. */
  extra_coals?: Coal[];
  /** 无报价也求解: extra_coals 按估算价并入煤池进 LP. */
  estimate_unpriced?: boolean;
  /** 煤价典型误差, 给价格稳健性分析用. */
  uncertainty?: PriceUncertainty | null;
  /** 基准报价 (带日期的市场价); 有就按同煤种、指标最接近的基准估价, 优先于 market_price. */
  benchmarks?: Benchmark[];
  /** 对标时外地基准的距离加罚 (点); 煤种、形态是硬分层, 地区只是软偏好. 缺省 = 不看地区. */
  region_penalty?: number | null;
}

/** 基准报价: 一个具体品种某天的市场价 (出厂含税). */
export interface Benchmark {
  name: string;
  coal_type?: string | null;
  /** 产品形态, 与 Coal.form 同一词表; 估价按 煤种 → 形态 → 地区 逐层对标. */
  form?: string | null;
  region?: string | null;
  /** 基准品的指标. */
  props: Partial<Record<string, number>>;
  /** 报价 (元/吨). */
  price: number;
  /** 报价日 YYYY-MM-DD. */
  date: string;
  source?: string | null;
  /** 按焦煤现货指数从报价日推算到今天的价 (服务端算). */
  current_price?: number | null;
}

/** /api/benchmarks 返回的基准: 多了出处层与指数推算信息. */
export interface BenchmarkEntry extends Benchmark {
  /** master = 随版本发布的默认值; stored = 管理员补录 (可删); mcci = 钢铁网指数当天值 (服务端每天拉). */
  origin?: "master" | "stored" | "mcci";
  id?: string;
  index_ratio?: number;
  index_date?: string;
  /** 推算用的指数: 同煤种指数序列的名字, 或 "CCP". */
  index_source?: string;
  /** origin=mcci 时的钢铁网品种代码. */
  index_code?: string;
}

/** master 里配置的钢铁网指数序列: 服务端每天拉值, 每条当一个基准 (指标 = 标准品). */
export interface IndexSeries {
  code: string;
  type_name: string;
  name: string;
  coal_type: string;
  form?: string;
  region?: string;
  props: Record<string, number>;
}

/** 钢铁网指数的一个交易日值 (元/湿吨). */
export interface McciPoint {
  date: string;
  price: number;
}

/** /api/mcci 的一条序列: master 配置 + 近一年的值 (升序, 可能为空). */
export interface McciSeries extends IndexSeries {
  points: McciPoint[];
}

/** 估价模块入口 (/api/price) 的输入. */
export interface PriceRequest {
  coals: Coal[];
  price_factor: PriceFactor;
}

/** 估价模块入口的输出. */
export interface PriceResult {
  ok: boolean;
  reason?: string | null;
  coal_values: CoalValue[];
  market_price?: number | null;
}

/** 煤价典型误差 (元/吨, 约等于平均绝对误差). */
export interface PriceUncertainty {
  /** 估算价, 煤种没单列时. */
  default: number;
  /** 有报价的煤. */
  quoted: number;
  /** 按煤种单列的估算价误差. */
  by_coal_type?: Record<string, number>;
}

/** master 里的价格因子系数表, 多一段出处说明. */
export interface MasterPriceFactor extends PriceFactor {
  description?: string;
  uncertainty_note?: string;
  benchmarks_note?: string;
  index_series_note?: string;
  /** 只给服务端用, 不进求解请求. */
  index_series?: IndexSeries[];
}

export interface PricePart {
  indicator: string;
  amount: number;
}

/** 单种煤的价格评估 (只用出厂价, 不含运费). gap > 0 偏贵, < 0 划算. */
export interface CoalValue {
  name: string;
  /** 出厂价; null = 没报价, 只有估算价. */
  fob: number | null;
  /** 质量升贴水合计 (点). */
  adjustment: number;
  /** 质量指数 = 1000 + 升贴水; 标准品 = 1000 点. */
  points: number;
  /** 应值 = 市场价/1000 × 质量指数 (元/吨). */
  fair: number;
  /** 没报价为 null. */
  gap: number | null;
  ratio: number | null;
  /** 配煤价值: 本次煤池与合同下的盈亏平衡到厂价 (元/吨); 到厂价低于它就会入选. */
  blend_value?: number | null;
  /** 估价对标的基准报价名; 缺省 = 按市场价或煤池反推. */
  benchmark?: string | null;
  parts: PricePart[];
  missing: string[];
  extrapolated: string[];
}

/** 价格稳健性报告: 煤价估不准时, 配方还靠不靠得住. */
export interface RobustnessReport {
  tolerance: number;
  swing_range: number;
  /** 配方用煤的平均估价误差 = Σ 配比 × 该煤估价误差 (元/吨); 不是吨成本误差. */
  price_error: number;
  /** 逐煤翻转门槛, 门槛小的在前. */
  swings: PriceSwing[];
  /** 更稳的配方; 缺省 = 当前配方已是最稳的. */
  robust?: RobustRecipe | null;
}

/** 单种煤价格变多少, 配方就要换掉一成以上吨位. */
export interface PriceSwing {
  name: string;
  coal_type?: string | null;
  /** 当前配方里的占比 (0–1); 没入选为 0. */
  share: number;
  /** 价格是不是估算价. */
  estimated: boolean;
  /** 这种煤的煤价误差 (元/吨). */
  uncertainty: number;
  /** 涨多少元配方会变 (5 元网格, 等于 5 = 不到 5 元); null = 搜索范围内不变或没入选. */
  rise: number | null;
  /** 跌多少元配方会变; null = 搜索范围内不变. */
  fall: number | null;
  /** 先问价: 估错就会改配方, 且按"占比 × 误差"排在最前的几种. */
  ask?: boolean;
}

/** 多花不超过容差、更多用价格有把握的煤的配方. */
export interface RobustRecipe {
  recipe: Record<string, number>;
  /** 按当前煤价的吨成本. */
  cost: number;
  /** 比最低成本配方多花多少 (元/吨). */
  extra_cost: number;
  /** 这个配方的煤价误差 (元/吨). */
  price_error: number;
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
  /** 按整铲配时这种煤配几铲; 未按铲配时没有. */
  shovels?: number | null;
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

/**
 * 不可行时的逐项诊断: 其余约束都成立的前提下, 该指标最好能做到多少.
 * 刻意不复用 IndicatorCheck —— 那里的 value 是"混合后的实际值", 不可行时没有配方.
 */
export interface InfeasibleBound {
  indicator: string;
  label_zh: string;
  direction: Direction;
  /** 合同要求的界 (Hard 用合同界, Priced 用拒收线). */
  required: number;
  /**
   * LP 实际执行的界: 合同界经判定规则折算, 再按方向扣掉安全余量 margin.
   * margin 会让它比合同界更严 (合同 ≤10 / 余量 0.5 ⇒ 按 ≤9.5 执行), 截断判定
   * 会让它更松 (合同 ≤10 / 一位小数截断 ⇒ 按 ≤10.0999 执行).
   * 「差多少」按两条界里更紧的那条算 —— 那才是真正卡住这一单的线.
   */
  enforced: number;
  /**
   * 本项的安全余量 (Spec.margin), 没配就是 0.
   * 用来区分执行界为什么比合同界紧: 余量是用户自己设的, 判定规则不是 —— 截断判定在
   * 下限一侧同样收紧 (合同 ≥14.95 按 ≥15.0 执行), 说成"含安全余量"就是编原因.
   */
  margin: number;
  /** 其余约束成立时该指标能达到的最优值. */
  achievable: number;
  /**
   * 合同上这个数改成多少就能求出配方 (Priced 指的是拒收线).
   * core 逐档真解验证过, 不是 achievable 与某条界的差值 —— 判定规则是阶梯函数,
   * 差值推出来的数往往落在同一档内, 改了等于没改.
   *
   * null = 逐档试下来没有一个数真能解出配方 (放宽这一项必要但不充分).
   * 这时界面只能说"非放宽它不可", 不能承诺某个数管用.
   */
  relax_to?: number | null;
}

export interface BlendResult {
  ok: boolean;
  reason?: string | null;
  /**
   * 不可行诊断: 每项都是"单独放宽它就能可行"的真凶.
   * 空 = 没有单独一项能解释 (冲突牵涉两项以上, 或煤池本身不够); 旧历史结果亦无此字段.
   */
  infeasible_bounds?: InfeasibleBound[];
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
  /** 捣固炼焦 CSR 估算 (由体检里的挥发/G/Y 算出); 只展示, 不参与求解. */
  csr_stamp_estimate?: number | null;
  /** 启用煤阶交互时: 本配方的煤阶方差 D (%²). */
  rank_variance?: number | null;
  /** 启用煤阶交互时: CSR 因此扣掉的点数 k·D (体检里的 CSR 已扣过). */
  csr_interaction_penalty?: number | null;
  /** 价格因子评估; 未配置或旧历史结果为空/缺失. */
  coal_values?: CoalValue[];
  /** 标准品 1000 点对应的市场价 (元/吨). */
  market_price?: number | null;
  market_price_from_group?: boolean;
  /** 价格稳健性分析; 请求没要或无解时缺失. */
  price_robustness?: RobustnessReport | null;
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
  /** 焦炭反应性与冷态强度 (%), 选填. */
  cri?: number | null;
  m40?: number | null;
  m10?: number | null;
  /** 炼焦条件, 选填: 装煤密度 (t/m³)、结焦时间 (小时)、炉温 (℃). */
  bulk_density?: number | null;
  coking_hours?: number | null;
  flue_temp?: number | null;
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
  /** 焦炭 CRI/M40/M10 与炼焦条件回填 (区分煤的原因和炉子的原因); null = 未回填. */
  cri_measured: number | null;
  m40_measured: number | null;
  m10_measured: number | null;
  bulk_density: number | null;
  coking_hours: number | null;
  flue_temp: number | null;
}

// ===== Master schema =====

export type CoalStatus = "verified" | "active" | "draft" | "incomplete" | "archived";
export type Confidence = "high" | "medium" | "low" | "spec";

export interface MasterCoalEntry {
  name: string;
  region?: string | null;
  coal_type?: string | null;
  /** 产品形态 (原煤/精煤/浮选精煤/中煤/煤泥), 与 coal_type 正交. */
  form?: string | null;
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

/** master 的受控词表声明. 键即允许取值, 值是该取值的含义. */
export interface MasterSchema {
  fields: Record<string, string>;
  coal_type: Record<string, string>;
  coal_type_basis?: string;
  form: Record<string, string>;
  status: Record<string, string>;
  confidence_per_field: Record<string, string>;
}

export interface CoalMaster {
  version: string;
  updated_at: string;
  description: string;
  /** 词表声明; 录入界面从这里取煤种/形态选项, 不在前端另写一份. */
  schema?: MasterSchema | null;
  default_contract: DefaultContract;
  /** 价格因子默认系数表; 求解时原样带进请求. */
  price_factor?: MasterPriceFactor | null;
  coals: MasterCoalEntry[];
}
