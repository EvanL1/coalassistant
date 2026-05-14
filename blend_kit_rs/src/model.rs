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
        }
    }
    pub fn lower(indicator: &str, min: f64) -> Self {
        Self {
            indicator: indicator.into(),
            direction: Direction::Lower,
            min: Some(min),
            max: None,
            enabled: true,
        }
    }
    pub fn range(indicator: &str, min: f64, max: f64) -> Self {
        Self {
            indicator: indicator.into(),
            direction: Direction::Range,
            min: Some(min),
            max: Some(max),
            enabled: true,
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
            warnings,
        }
    }
}
