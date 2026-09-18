//! 数据模型: 8 项指标 + FOB + FRT 的 Coal, 8 条 Spec, 三视图输出.
//!
//! 设计原则:
//!   - 每字段一个来源 (化验/经验/采购/物流)
//!   - 派生量 (CIF) 用函数不用字段
//!   - LP 输入是不可变快照
//!   - 输出分三视图: 成本结构 / 实物订单 / 指标体检
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 8 项指标的标识符. 与 UI 中文标签一一对应:
///   "S"     → 硫
///   "A"     → 灰
///   "V"     → 挥发
///   "G"     → 粘结
///   "Y"     → 胶质
///   "petro" → 岩相
///   "CSR"   → 焦炭强度
///   "M"     → 水分
pub const INDICATORS: [&str; 8] = ["S", "A", "V", "G", "Y", "petro", "CSR", "M"];

/// 中文标签查询表 (给 UI 用).
pub fn label_zh(key: &str) -> &'static str {
    match key {
        "S" => "硫",
        "A" => "灰",
        "V" => "挥发",
        "G" => "粘结",
        "Y" => "胶质",
        "petro" => "岩相",
        "CSR" => "焦炭强度",
        "M" => "水分",
        _ => "未知",
    }
}

/// 一种煤的化验数据 + 经验值 + 价格.
///
/// 字段来源(责任部门):
///   props 中的 S/A/V/G/Y/M    -> 化验单 (质检)
///   props 中的 petro/CSR      -> 经验值 (技术)
///   fob                       -> 报价单 (采购)
///   frt                       -> 物流报价 (物流)
///
/// props 用 HashMap 是为了**指标缺失时跳过**, 而不是为了运行时加新指标.
/// 8 项指标是固定集合, 见 INDICATORS 常量.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coal {
    pub name: String,
    /// 8 项指标的实测/经验值. 缺失项不放.
    pub props: HashMap<String, f64>,
    /// 出厂价 (FOB), 元/吨.
    pub fob: f64,
    /// 运费 (FRT), 元/吨.
    pub frt: f64,
    /// 可选煤岩数据 (MT/T 507 化验单: 反射率直方图 + 镜质组含量).
    /// 提供时: 混煤 σ 走直方图精确计算; props 缺 petro 标量时自动用直方图 σ 补齐.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub petrography: Option<crate::petrography::Petrography>,
    /// 该煤采购合同条款. None = 按报价原值, 不做买入侧修正.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase_terms: Option<PurchaseTerms>,
}

impl Coal {
    /// 到厂价 = 出厂价 + 运费. 派生函数, 不存字段.
    pub fn cif(&self) -> f64 {
        self.fob + self.frt
    }

    pub fn has(&self, indicator: &str) -> bool {
        self.props.contains_key(indicator)
    }

    pub fn get(&self, indicator: &str) -> Option<f64> {
        self.props.get(indicator).copied()
    }
}

/// 合同约束的方向.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Direction {
    /// 越小越好, 只有 max 起作用 (硫/灰/水分典型)
    Upper,
    /// 越大越好, 只有 min 起作用 (粘结/胶质/焦炭强度典型)
    Lower,
    /// 目标范围, min 和 max 都起作用 (挥发/反射率典型)
    Range,
}

/// 扣款档位: 从上一档终点起, 覆盖 width 宽度的偏离, 按 rate 计费.
///
/// 合同写"每超 0.1% 扣 8 元/吨"时, rate = 80.0 (元/吨 per 1 个指标单位).
/// 单位换算由前端完成, core 只收换算后的斜率.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PenaltyTier {
    /// 本档覆盖的偏离宽度 (指标单位). None = 末档, 无上限.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// 元/吨 per 1 个指标单位.
    pub rate: f64,
}

/// 单项指标的计价条款.
///
/// tiers 的 rate 必须严格递增: 凸性是"档位变量可用 LP 精确表达"的前提,
/// 递减的 rate 会让求解器填错档并低估扣款.
///
/// 正负号由调用方负责: 买入侧扣款是折扣 (降低成本), 卖出侧扣款是费用 (提高成本);
/// `reject` 的方向 (超上限拒收还是低于下限拒收) 由外层 `Spec`/`PurchaseClause`
/// 的 `direction` 决定, 不由 `Penalty` 自身判断.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Penalty {
    pub tiers: Vec<PenaltyTier>,
    /// 拒收线: 越过即硬不可行. Upper 向是上限, Lower 向是下限.
    pub reject: f64,
}

/// 单条采购合同计价条款 (买入侧).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PurchaseClause {
    pub indicator: String,
    /// 只接受 Upper (超标扣) / Lower (低于扣).
    pub direction: Direction,
    /// 该煤采购合同的保证值.
    pub guarantee: f64,
    pub penalty: Penalty,
}

/// 采购合同条款. 前端已把全局模板与单煤覆盖合并完毕后传入.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PurchaseTerms {
    pub clauses: Vec<PurchaseClause>,
    /// 合同水分 (%), 用于结算量折算. None = 不折算.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_moisture: Option<f64>,
    /// 超过该水分 (%) 时, 超出部分按 2 倍计入有效水分 M_eff. None = 不启用.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moisture_excess_double_threshold: Option<f64>,
}

/// 合同值采用的报告判定方式.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum AcceptanceMode {
    #[default]
    Raw,
    Truncate,
    Round,
}

/// 业务判定规则. `tolerance` 是合同允许的向外偏差，不是模型安全余量.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptanceRule {
    #[serde(default)]
    pub mode: AcceptanceMode,
    #[serde(default)]
    pub decimals: Option<u8>,
    #[serde(default)]
    pub tolerance: f64,
}

impl Default for AcceptanceRule {
    fn default() -> Self {
        Self {
            mode: AcceptanceMode::Raw,
            decimals: None,
            tolerance: 0.0,
        }
    }
}

/// 约束在优化中的执行强度.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Enforcement {
    /// 进入 LP；无法满足时不返回配方.
    #[default]
    Hard,
    /// 不阻断求解，但超界会将总体质量状态降为 NeedsReview.
    Soft,
    /// 只展示结果，不影响总体质量状态.
    Advisory,
    /// 进入 LP, 但超界不判不可行, 而是按 Penalty 折算成元/吨计入目标;
    /// 越过 Penalty.reject 仍然硬不可行.
    Priced,
}

/// 单条合同约束.
///
/// 设计:
///   enabled = false → LP 完全跳过此约束
///   Hard 且煤池缺输入 → 自动剔除缺输入的煤, 加 warning
///   Soft/Advisory → 不改变候选煤池
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub indicator: String,
    pub direction: Direction,
    pub min: Option<f64>,
    pub max: Option<f64>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 安全余量: LP 内部把上限收紧 margin、下限抬高 margin, 输出展示仍用合同原界限.
    /// 用途: 兜住"LP 解必然贴边 + G 值加权系统性高估实测 ~11%"的双重风险
    /// (调研 2026-07-04 §2). None = 不收紧.
    #[serde(default)]
    pub margin: Option<f64>,
    /// 逐指标业务判定规则. None 时兼容 `BlendRequest.truncate_decimal`.
    #[serde(default)]
    pub acceptance: Option<AcceptanceRule>,
    /// Hard/Soft/Advisory. 旧请求默认为 Hard.
    #[serde(default)]
    pub enforcement: Enforcement,
    /// enforcement == Priced 时必填的计价条款.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub penalty: Option<Penalty>,
}

fn default_enabled() -> bool {
    true
}

impl Spec {
    pub fn upper(indicator: &str, max: f64) -> Self {
        Self {
            indicator: indicator.into(),
            direction: Direction::Upper,
            min: None,
            max: Some(max),
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Hard,
            penalty: None,
        }
    }
    pub fn lower(indicator: &str, min: f64) -> Self {
        Self {
            indicator: indicator.into(),
            direction: Direction::Lower,
            min: Some(min),
            max: None,
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Hard,
            penalty: None,
        }
    }
    pub fn range(indicator: &str, min: f64, max: f64) -> Self {
        Self {
            indicator: indicator.into(),
            direction: Direction::Range,
            min: Some(min),
            max: Some(max),
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Hard,
            penalty: None,
        }
    }
}

/// 一次混煤的线性 G 代理与实测 G 对照.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GObservation {
    pub g_linear: f64,
    pub g_measured: f64,
}

/// 可选评估器的启用门槛. 样本数只是底线，最终门控使用交叉验证误差.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPolicy {
    pub min_g_samples: usize,
    pub min_csr_samples: usize,
    pub max_g_cv_mae: f64,
    pub max_csr_cv_mae: f64,
    pub extrapolation_ratio: f64,
    pub ridge_lambda: f64,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            min_g_samples: 20,
            min_csr_samples: 30,
            max_g_cv_mae: 4.0,
            max_csr_cv_mae: 5.0,
            extrapolation_ratio: 0.1,
            ridge_lambda: 1e-3,
        }
    }
}

/// 求解输入 (来自前端 JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendRequest {
    pub coals: Vec<Coal>,
    pub specs: Vec<Spec>,
    /// 单次采购总吨数. None 时输出只含比例和单位成本, 不算实物订单.
    pub total_quantity: Option<f64>,
    /// 是否启用一位小数截断规则.
    #[serde(default = "default_truncate")]
    pub truncate_decimal: bool,
}

fn default_truncate() -> bool {
    true
}

// ============================================================================
// 输出: 三视图
// ============================================================================

/// 视图 A: 成本三层结构 (给财务).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub fob_per_ton: f64,
    pub frt_per_ton: f64,
    /// 到厂价. 扣款条款落地后仅作展示用, LP 实际优化的是 `net_per_ton`.
    pub cif_per_ton: f64,
    /// 仅当 total_quantity 提供时填充.
    pub total_fob: Option<f64>,
    pub total_frt: Option<f64>,
    pub total_cif: Option<f64>,
    /// 买入侧扣款折扣 + 水分折算带来的到厂价修正合计, 元/吨. 负值 = 成本下降.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
    #[serde(default)]
    pub purchase_adjust_per_ton: f64,
    /// 卖出侧质量扣款合计, 元/吨.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
    #[serde(default)]
    pub penalty_per_ton: f64,
    /// 真实吨成本 = cif + purchase_adjust + penalty.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
    #[serde(default)]
    pub net_per_ton: f64,
    pub total_purchase_adjust: Option<f64>,
    pub total_penalty: Option<f64>,
    pub total_net: Option<f64>,
}

/// 视图 B: 单条实物订单 (给采购).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderItem {
    pub coal: String,
    /// LP 求出的配比 x_i*.
    pub ratio: f64,
    /// 仅当 total_quantity 提供时填充.
    pub tons: Option<f64>,
    pub fob_amount: Option<f64>,
    pub frt_amount: Option<f64>,
    pub cif_amount: Option<f64>,
    /// 该煤买入侧修正后的单价 (元/吨), 采购按此价核对.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 OrderItem 记录 (无此字段).
    #[serde(default)]
    pub cif_eff_per_ton: f64,
}

/// 视图 C: 单项指标的体检结果 (给质检/销售).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorCheck {
    pub indicator: String,
    pub label_zh: String,
    /// 混合后的实际值.
    pub value: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// 距离最近边界的余量. None 表示该指标无约束 (Spec 未配置或未启用).
    /// 负值表示已违反约束.
    pub slack: Option<f64>,
    /// 是否 binding (顶格): slack 接近 0 且非负.
    /// binding 集合是谈判方向的逆向归因依据.
    pub binding: bool,
    /// LP 使用的线性代理值.
    pub proxy_value: Option<f64>,
    /// 经可选评估器修正后的值. `value` 保留为同值以兼容旧调用方.
    pub evaluated_value: Option<f64>,
    /// 按合同截断/四舍五入规则得到的判定值.
    pub judged_value: Option<f64>,
    /// 模型的 P90 绝对误差；无已验证模型时为 None.
    pub uncertainty: Option<f64>,
    #[serde(default)]
    pub method: EvaluationMethod,
    #[serde(default)]
    pub status: EvaluationStatus,
    pub model: Option<ModelSummary>,
    /// 本项卖出侧扣款, 元/吨. 非计价指标为 None.
    pub penalty_per_ton: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvaluationMethod {
    #[default]
    Linear,
    ProvisionalLinear,
    AffineCalibration,
    Histogram,
    Moments,
    Regression,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvaluationStatus {
    #[default]
    Pass,
    TolerancePass,
    Unverified,
    Fail,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum QualityStatus {
    Verified,
    #[default]
    Estimated,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModelKind {
    GAffine,
    CsrRidge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSummary {
    pub version: String,
    pub kind: ModelKind,
    pub sample_count: usize,
    pub cv_mae: f64,
    pub p90_abs_error: f64,
    pub bias: f64,
    pub in_domain: bool,
}

/// 岩相校验 (视图 C 补充): 按直方图或 μ/σ 计算的混煤反射率分布指标.
/// 只有参配煤全部带有效煤岩输入时才产出；与 indicator_check 里的线性代理值
/// (Σx·σ_j) 不同，这里的 sigma 含 μ 离散贡献 (全方差定律).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetrographyCheck {
    /// 混煤反射率均值.
    pub mean: f64,
    /// 混煤反射率标准差 (精确值).
    pub sigma: f64,
    /// 合同 σ 上限 (petro spec 的 max). 无约束时 None.
    pub sigma_max: Option<f64>,
    /// σ 是否满足上限. 无约束时 None.
    pub sigma_ok: Option<bool>,
    /// 主焦区间 (1.2~1.5) 凹口. None = 无凹口.
    pub notch: Option<crate::petrography::Notch>,
    /// 线性代理收紧迭代次数 (0 = 一次通过).
    pub refine_iterations: usize,
}

/// 完整求解结果.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlendResult {
    pub ok: bool,
    pub reason: Option<String>,
    /// 配方: 煤名 → 配比. 仅含 > 1e-5 的煤.
    pub recipe: HashMap<String, f64>,
    /// 视图 A.
    pub cost: Option<CostBreakdown>,
    /// 视图 B (按配比降序).
    pub orders: Vec<OrderItem>,
    /// 视图 C (按 INDICATORS 顺序).
    pub indicator_check: Vec<IndicatorCheck>,
    /// 岩相精确校验. 参配煤缺煤岩数据时为 None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub petrography_check: Option<PetrographyCheck>,
    /// 容错过程中的警告.
    pub warnings: Vec<String>,
    /// 求解成功与质量可信度分离: ok=true 仍可能是 Estimated/NeedsReview.
    #[serde(default)]
    pub quality_status: QualityStatus,
    /// 求解后评估和修复的重算次数.
    #[serde(default)]
    pub evaluation_iterations: usize,
}

impl BlendResult {
    pub fn infeasible(reason: &str, warnings: Vec<String>) -> Self {
        Self {
            ok: false,
            reason: Some(reason.into()),
            recipe: HashMap::new(),
            cost: None,
            orders: Vec::new(),
            indicator_check: Vec::new(),
            petrography_check: None,
            warnings,
            quality_status: QualityStatus::NeedsReview,
            evaluation_iterations: 0,
        }
    }
}
