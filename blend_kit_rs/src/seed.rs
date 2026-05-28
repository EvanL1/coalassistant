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
        let json = include_str!("../data/coal_master.json");
        serde_json::from_str(json).map_err(|e| format!("master JSON 解析失败: {}", e))
    }

    pub fn find(&self, name: &str) -> Option<&CoalMasterEntry> {
        self.coals.iter().find(|c| c.name == name)
    }

    pub fn by_status<'a>(
        &'a self,
        status: MasterStatus,
    ) -> impl Iterator<Item = &'a CoalMasterEntry> {
        self.coals.iter().filter(move |c| c.status == status)
    }

    pub fn verified(&self) -> impl Iterator<Item = &CoalMasterEntry> {
        self.by_status(MasterStatus::Verified)
    }
}

impl CoalMasterEntry {
    /// 是否有 S/A/V/G 基础四项.
    pub fn has_basic(&self) -> bool {
        ["S", "A", "V", "G"].iter().all(|k| self.props.contains_key(*k))
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
    pub fn to_coal(&self, fob_override: Option<f64>, frt_override: Option<f64>) -> Option<crate::Coal> {
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
        })
    }

    /// 检查某字段是否是低可信度估值.
    pub fn is_low_confidence(&self, field: &str) -> bool {
        matches!(self.confidence.get(field), Some(Confidence::Low))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_embedded() {
        let master = CoalMaster::load_embedded().expect("master 加载失败");
        assert!(!master.coals.is_empty());
        assert_eq!(master.version, "2.1");
        assert!(
            master.coals.len() >= 60,
            "煤种数 {} 异常",
            master.coals.len()
        );
    }

    /// 4 种主力煤都应该是 verified + production_ready.
    #[test]
    fn test_main_coals_production_ready() {
        let master = CoalMaster::load_embedded().unwrap();
        for name in &["临北", "古交浮精", "豹子沟", "大佛寺"] {
            let entry = master.find(name).expect(name);
            assert_eq!(entry.status, MasterStatus::Verified, "{} 应为 verified", name);
            assert!(entry.is_production_ready(), "{} 应有完整 10 字段", name);
            assert!(entry.has_full_indicators(), "{} 应有 8 项指标", name);
        }
    }

    /// 临北字段精确值检查.
    #[test]
    fn test_linbei_exact_values() {
        let master = CoalMaster::load_embedded().unwrap();
        let lb = master.find("临北").unwrap();
        assert_eq!(lb.props.get("S"), Some(&2.0));
        assert_eq!(lb.props.get("A"), Some(&6.0));
        assert_eq!(lb.props.get("V"), Some(&22.0));
        assert_eq!(lb.props.get("G"), Some(&93.0));
        assert_eq!(lb.props.get("Y"), Some(&17.0));
        assert_eq!(lb.props.get("petro"), Some(&0.08));
        assert_eq!(lb.props.get("CSR"), Some(&70.0));
        assert_eq!(lb.props.get("M"), Some(&11.0));
        assert_eq!(lb.fob, Some(1425.0));
        assert_eq!(lb.frt, Some(25.0));
        // 可信度: S/A/V/G/CSR/fob/frt 高, petro 低
        assert_eq!(lb.confidence.get("S"), Some(&Confidence::High));
        assert_eq!(lb.confidence.get("petro"), Some(&Confidence::Low));
        assert!(lb.is_low_confidence("petro"));
        assert!(!lb.is_low_confidence("S"));
    }

    /// to_coal 转换 + CIF 计算.
    #[test]
    fn test_linbei_to_coal_uses_master_price() {
        let master = CoalMaster::load_embedded().unwrap();
        let lb = master.find("临北").unwrap();
        let coal = lb.to_coal(None, None).unwrap();
        assert_eq!(coal.fob, 1425.0);
        assert_eq!(coal.frt, 25.0);
        assert_eq!(coal.cif(), 1450.0);

        // override 覆盖 master 价
        let coal2 = lb.to_coal(Some(1300.0), None).unwrap();
        assert_eq!(coal2.fob, 1300.0);
        assert_eq!(coal2.frt, 25.0); // frt 仍走 master
    }

    /// 默认合同模板含 8 条 spec.
    #[test]
    fn test_default_contract() {
        let master = CoalMaster::load_embedded().unwrap();
        let c = &master.default_contract;
        assert_eq!(c.specs.len(), 8);
        // 验证关键约束
        let s_spec = c.specs.iter().find(|s| s.indicator == "S").unwrap();
        assert_eq!(s_spec.max, Some(2.5));
        let v_spec = c.specs.iter().find(|s| s.indicator == "V").unwrap();
        // 用户最新合同: V ≤ 23 (Upper), 不是 Range
        assert_eq!(v_spec.max, Some(23.0));
        let csr_spec = c.specs.iter().find(|s| s.indicator == "CSR").unwrap();
        assert_eq!(csr_spec.min, Some(62.0));
    }

    /// incomplete 煤无法转 Coal.
    #[test]
    fn test_incomplete_cannot_convert() {
        let master = CoalMaster::load_embedded().unwrap();
        let xjg = master.find("兴家沟").unwrap();
        assert_eq!(xjg.status, MasterStatus::Incomplete);
        assert!(!xjg.has_basic());
        assert!(xjg.to_coal(Some(1000.0), Some(30.0)).is_none());
    }

    /// 状态分布健全.
    #[test]
    fn test_status_distribution() {
        let master = CoalMaster::load_embedded().unwrap();
        let verified = master.verified().count();
        let active = master.by_status(MasterStatus::Active).count();
        let draft = master.by_status(MasterStatus::Draft).count();
        let incomplete = master.by_status(MasterStatus::Incomplete).count();
        let archived = master.by_status(MasterStatus::Archived).count();
        assert_eq!(verified, 4, "verified 应正好 4 种 (主力煤): 实际 {}", verified);
        assert!(active >= 50, "active 应 ≥ 50");
        assert!(draft >= 2, "draft 应包含沙曲/贺西等");
        assert!(incomplete >= 5);
        assert!(archived >= 1, "archived 应含古交-原");
    }
}
