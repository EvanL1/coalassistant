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

/// 单条合同约束.
///
/// 设计:
///   enabled = false → LP 完全跳过此约束
///   enabled = true 但煤池缺数据 → 自动剔除缺数据的煤, 加 warning
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
    /// 可选: 历史 [混合指标 → 实测 CSR] 观测.
    /// 提供且样本足够时, 用线性回归预测覆盖各煤 CSR; 不提供则保持各煤录入的 CSR.
    #[serde(default)]
    pub csr_observations: Option<Vec<crate::predict::CsrObservation>>,
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
    pub cif_per_ton: f64,
    /// 仅当 total_quantity 提供时填充.
    pub total_fob: Option<f64>,
    pub total_frt: Option<f64>,
    pub total_cif: Option<f64>,
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
}

/// 岩相校验 (视图 C 补充): 按直方图合成精确计算的混煤反射率分布指标.
/// 只有参配煤全部带煤岩数据时才产出; 与 indicator_check 里的线性代理值 (Σx·σ_j) 不同,
/// 这里的 sigma 含 μ 离散贡献 (全方差定律), 是可对照化验单的真实值.
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
        }
    }
}
