//! 煤种 master 数据库 (v2).
//!
//! 设计:
//!   - master 提供完整 10 字段 (S/A/V/G/Y/petro/CSR/M/fob/frt) + per-field 可信度
//!   - status: verified (生产可用) / active (部分数据) / draft (待核实) / incomplete / archived
//!   - 用户首次启动 APP 时把 master 全量 seed 到本地 SQLite
//!   - master 升级时 APP 只 insert 新增项, 不覆盖用户的 override
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// per-field 可信度.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// 用户直接录入的实测/报价值
    High,
    /// 用户修正过的估值, 有事实依据
    Medium,
    /// 纯估值, 行业典型值或推测
    Low,
}

/// 煤种状态.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MasterStatus {
    /// 全部 10 字段已核实, 生产配煤直接可用
    Verified,
    /// S/A/V/G 4 字段实测, 其余待补
    Active,
    /// 数据有疑点, 启用前需用户确认
    Draft,
    /// 煤名已知, 化验数据未录入
    Incomplete,
    /// 已停用 (来源缺货/煤种不符)
    Archived,
}

/// master 中一条煤种记录.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoalMasterEntry {
    pub name: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub coal_type: Option<String>,
    pub status: MasterStatus,
    /// 8 项化验指标, 缺失项不放
    #[serde(default)]
    pub props: HashMap<String, f64>,
    /// 出厂价 (master 中保存最近一次报价, 但用户应自己输入今日价)
    #[serde(default)]
    pub fob: Option<f64>,
    /// 运费
    #[serde(default)]
    pub frt: Option<f64>,
    /// per-field 可信度 (仅对 verified/active 的煤有意义)
    #[serde(default)]
    pub confidence: HashMap<String, Confidence>,
    #[serde(default)]
    pub note: Option<String>,
}

/// master 默认合同模板.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultContract {
    pub name: String,
    pub specs: Vec<crate::Spec>,
}

/// master 数据库根结构.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoalMaster {
    pub version: String,
    pub updated_at: String,
    pub description: String,
    pub default_contract: DefaultContract,
    pub coals: Vec<CoalMasterEntry>,
}

impl CoalMaster {
    /// 加载嵌入到二进制中的 master JSON.
    pub fn load_embedded() -> Result<Self, String> {
        serde_json::from_str(crate::master_json())
            .map_err(|e| format!("master JSON 解析失败: {}", e))
    }

    pub fn find(&self, name: &str) -> Option<&CoalMasterEntry> {
        self.coals.iter().find(|c| c.name == name)
    }

    pub fn by_status(&self, status: MasterStatus) -> impl Iterator<Item = &CoalMasterEntry> {
        self.coals.iter().filter(move |c| c.status == status)
    }

    pub fn verified(&self) -> impl Iterator<Item = &CoalMasterEntry> {
        self.by_status(MasterStatus::Verified)
    }
}

impl CoalMasterEntry {
    /// 是否有 S/A/V/G 基础四项.
    pub fn has_basic(&self) -> bool {
        ["S", "A", "V", "G"]
            .iter()
            .all(|k| self.props.contains_key(*k))
    }

    /// 是否有完整 8 项化验指标 (含 Y/petro/CSR/M).
    pub fn has_full_indicators(&self) -> bool {
        ["S", "A", "V", "G", "Y", "petro", "CSR", "M"]
            .iter()
            .all(|k| self.props.contains_key(*k))
    }

    /// 是否所有字段 (含 fob/frt) 都齐.
    pub fn is_production_ready(&self) -> bool {
        self.has_full_indicators() && self.fob.is_some() && self.frt.is_some()
    }

    /// 转换成 LP 用的 Coal.
    /// - 用 master 内的 fob/frt (若有)
    /// - 否则要求调用者传入 (fob_override, frt_override)
    pub fn to_coal(
        &self,
        fob_override: Option<f64>,
        frt_override: Option<f64>,
    ) -> Option<crate::Coal> {
        if !self.has_basic() {
            return None;
        }
        let fob = fob_override.or(self.fob)?;
        let frt = frt_override.or(self.frt)?;
        Some(crate::Coal {
            name: self.name.clone(),
            props: self.props.clone(),
            fob,
            frt,
            petrography: None, // master 数据尚无煤岩直方图字段
        })
    }

    /// 检查某字段是否是低可信度估值.
    pub fn is_low_confidence(&self, field: &str) -> bool {
        matches!(self.confidence.get(field), Some(Confidence::Low))
    }
}

/// 这里的测试只覆盖**代码行为**, 一律用 fixture 构造输入.
///
/// 具体煤源的字段值、座数、状态分布、版本号属于**数据内容**, 由
/// `scripts/check_master_data.mjs` 负责校验 —— 更新煤库数据不应该让这里变红.
/// 唯一碰真实 master 的是 `test_embedded_master_deserializes`, 它测的也只是
/// "嵌入的 JSON 接得上当前 struct 定义", 与具体数值无关.
#[cfg(test)]
mod tests {
    use super::*;

    /// 按 JSON 造一条 master 记录, 避免为测试给 struct 开构造器.
    fn entry(json: &str) -> CoalMasterEntry {
        serde_json::from_str(json).expect("fixture 解析失败")
    }

    /// 10 字段俱全的样板煤.
    fn full_entry() -> CoalMasterEntry {
        entry(
            r#"{
                "name": "样板煤", "status": "verified",
                "props": { "S": 2.0, "A": 6.0, "V": 22, "G": 93,
                           "Y": 17, "petro": 0.08, "CSR": 70, "M": 11 },
                "fob": 1425, "frt": 25,
                "confidence": { "S": "high", "petro": "low" }
            }"#,
        )
    }

    /// 嵌入的 master JSON 必须能反序列化成当前 struct —— 只测 schema 对得上.
    #[test]
    fn test_embedded_master_deserializes() {
        let master = CoalMaster::load_embedded().expect("master 加载失败");
        assert!(!master.coals.is_empty(), "coals 不应为空");
        assert!(
            !master.default_contract.specs.is_empty(),
            "默认合同不应为空"
        );
    }

    /// has_basic 只认 S/A/V/G 四项.
    #[test]
    fn test_has_basic_requires_savg() {
        assert!(full_entry().has_basic());

        let missing_g = entry(
            r#"{ "name": "缺G", "status": "active",
                 "props": { "S": 1.0, "A": 9.0, "V": 20 } }"#,
        );
        assert!(!missing_g.has_basic(), "缺 G 不应算基础齐全");
        assert!(!missing_g.has_full_indicators());
    }

    /// has_full_indicators 要 8 项; is_production_ready 还要 fob/frt.
    #[test]
    fn test_production_ready_needs_indicators_and_prices() {
        let full = full_entry();
        assert!(full.has_full_indicators());
        assert!(full.is_production_ready());

        let no_price = entry(
            r#"{ "name": "无价", "status": "active",
                 "props": { "S": 2.0, "A": 6.0, "V": 22, "G": 93,
                            "Y": 17, "petro": 0.08, "CSR": 70, "M": 11 } }"#,
        );
        assert!(no_price.has_full_indicators(), "8 项指标是齐的");
        assert!(!no_price.is_production_ready(), "缺 fob/frt 不算可直接生产");
    }

    /// to_coal 的闸门是 has_basic, 不是 status —— 缺基础指标一律转不出来.
    #[test]
    fn test_to_coal_rejects_without_basic_indicators() {
        let bare = entry(r#"{ "name": "待录入", "status": "incomplete", "props": {} }"#);
        assert!(!bare.has_basic());
        assert!(
            bare.to_coal(Some(1000.0), Some(30.0)).is_none(),
            "即便外部给了价格, 缺 S/A/V/G 也不能进煤池"
        );
    }

    /// 缺价且外部不补价时也转不出来.
    #[test]
    fn test_to_coal_requires_price_from_somewhere() {
        let no_price = entry(
            r#"{ "name": "无价", "status": "active",
                 "props": { "S": 2.0, "A": 6.0, "V": 22, "G": 93 } }"#,
        );
        assert!(no_price.to_coal(None, None).is_none(), "无价应转不出");
        assert!(
            no_price.to_coal(Some(1200.0), Some(30.0)).is_some(),
            "外部补价后应可转"
        );
    }

    /// to_coal 价格优先级: override > master, 且两个价互不影响.
    #[test]
    fn test_to_coal_price_override_precedence() {
        let e = full_entry();

        let coal = e.to_coal(None, None).unwrap();
        assert_eq!(coal.fob, 1425.0);
        assert_eq!(coal.frt, 25.0);
        assert_eq!(coal.cif(), 1450.0, "cif 应为 fob+frt");

        let overridden = e.to_coal(Some(1300.0), None).unwrap();
        assert_eq!(overridden.fob, 1300.0, "fob 应被覆盖");
        assert_eq!(overridden.frt, 25.0, "frt 未覆盖时仍走 master");

        let both = e.to_coal(Some(1300.0), Some(40.0)).unwrap();
        assert_eq!(both.cif(), 1340.0);
    }

    /// is_low_confidence 只对显式标 low 的字段为真.
    #[test]
    fn test_is_low_confidence() {
        let e = full_entry();
        assert!(e.is_low_confidence("petro"), "petro 标了 low");
        assert!(!e.is_low_confidence("S"), "S 标的是 high");
        assert!(!e.is_low_confidence("CSR"), "CSR 没标, 不算 low");
    }

    /// by_status / verified / find 的过滤行为.
    #[test]
    fn test_lookup_and_status_filters() {
        let master: CoalMaster = serde_json::from_str(
            r#"{
                "version": "9.9", "updated_at": "2026-01-01", "description": "fixture",
                "default_contract": { "name": "测试合同", "specs": [
                    { "indicator": "S", "direction": "Upper", "max": 2.5 } ] },
                "coals": [
                    { "name": "甲", "status": "verified", "props": {} },
                    { "name": "乙", "status": "active",   "props": {} },
                    { "name": "丙", "status": "active",   "props": {} },
                    { "name": "丁", "status": "archived", "props": {} }
                ]
            }"#,
        )
        .expect("fixture master 解析失败");

        assert_eq!(master.find("乙").map(|c| c.name.as_str()), Some("乙"));
        assert!(master.find("不存在").is_none());
        assert_eq!(master.verified().count(), 1);
        assert_eq!(master.by_status(MasterStatus::Active).count(), 2);
        assert_eq!(master.by_status(MasterStatus::Archived).count(), 1);
        assert_eq!(master.by_status(MasterStatus::Incomplete).count(), 0);
    }
}
