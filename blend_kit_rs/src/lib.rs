//! 豆哥配煤 - 配煤优化核心算法.
//!
//! 数据流程:
//!   业务侧 4 张表 (化验/合同/煤价/物流) → 归一为 COALS + SPECS
//!   → LP 建模 (最小化 CIF 加权和 + 8 条加权约束)
//!   → Clarabel 求解
//!   → 后处理为 3 个业务视图 (成本结构 / 实物订单 / 指标体检)
pub mod model;
pub mod optimizer;
pub mod predict;
pub mod seed;
pub use predict::{CsrObservation, CsrPredictor};
pub use seed::{CoalMaster, CoalMasterEntry, MasterStatus, Confidence, DefaultContract};

pub use model::*;
pub use optimizer::solve;

/// JSON in/out 入口 (前端通过此函数调用).
pub fn solve_json(input_json: &str) -> String {
    let result = match serde_json::from_str::<BlendRequest>(input_json) {
        Ok(req) => solve(&req),
        Err(e) => BlendResult::infeasible(&format!("JSON 解析失败: {}", e), Vec::new()),
    };
    serde_json::to_string(&result).unwrap_or_else(|_| {
        r#"{"ok":false,"reason":"序列化失败"}"#.to_string()
    })
}

/// 构造便捷函数: 用 10 字段元组造一个 Coal.
/// 顺序: (S, A, V, G, Y, petro, CSR, M, FOB, FRT)
pub fn coal_from_tuple(name: &str, t: (f64, f64, f64, f64, f64, f64, f64, f64, f64, f64)) -> Coal {
    let (s, a, v, g, y, petro, csr, m, fob, frt) = t;
    let mut props = std::collections::HashMap::new();
    props.insert("S".into(), s);
    props.insert("A".into(), a);
    props.insert("V".into(), v);
    props.insert("G".into(), g);
    props.insert("Y".into(), y);
    props.insert("petro".into(), petro);
    props.insert("CSR".into(), csr);
    props.insert("M".into(), m);
    Coal {
        name: name.into(),
        props,
        fob,
        frt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用你昨晚验证过的临北数据.
    #[test]
    fn test_linbei_tuple() {
        let linbei = coal_from_tuple("临北", (2.0, 6.0, 22.0, 93.0, 17.0, 0.01, 70.0, 11.0, 1425.0, 25.0));
        assert_eq!(linbei.fob, 1425.0);
        assert_eq!(linbei.frt, 25.0);
        assert_eq!(linbei.cif(), 1450.0);
        assert_eq!(linbei.get("S"), Some(2.0));
        assert_eq!(linbei.get("M"), Some(11.0));
    }

    #[test]
    fn test_basic_solve() {
        // 8 煤池, 数据贴近你之前的样例 (加 FOB/FRT 拆分 + petro/CSR 估值)
        let coals = vec![
            coal_from_tuple("铁新",   (3.2, 6.5,  21.0, 92.0,  14.0, 0.10, 62.0, 7.5, 1260.0, 30.0)),
            coal_from_tuple("临北",   (2.0, 6.0,  22.0, 93.0,  16.0, 0.10, 66.0, 7.0, 1250.0, 30.0)),
            coal_from_tuple("安益",   (3.4, 6.8,  18.0, 75.0,  10.0, 0.18, 60.0, 8.0, 1120.0, 30.0)),
            coal_from_tuple("筛精",   (3.9, 9.5,  25.0, 100.0, 22.0, 0.08, 65.0, 9.5, 970.0,  30.0)),
            coal_from_tuple("孟子峪", (2.0, 9.5,  18.0, 75.0,  10.0, 0.18, 58.0, 7.8, 1034.0, 30.0)),
            coal_from_tuple("大佛寺", (3.0, 8.5,  18.0, 75.0,  10.0, 0.16, 59.0, 8.0, 1120.0, 30.0)),
            coal_from_tuple("神州",   (2.6, 10.0, 17.0, 65.0,   8.0, 0.20, 55.0, 7.2, 1110.0, 30.0)),
            coal_from_tuple("豹子沟", (3.8, 11.0, 24.0, 92.0,  20.0, 0.12, 64.0, 8.5, 1250.0, 30.0)),
        ];
        let specs = vec![
            Spec::upper("S", 2.5),
            Spec::upper("A", 9.0),
            Spec::range("V", 18.0, 27.0),
            Spec::lower("G", 80.0),
            Spec::lower("Y", 14.0),
        ];
        let req = BlendRequest {
            coals,
            specs,
            total_quantity: Some(3700.0),
            truncate_decimal: true,
            csr_observations: None,
        };
        let r = solve(&req);
        assert!(r.ok, "expected feasible: {:?}", r.reason);

        let cost = r.cost.unwrap();
        assert!(cost.cif_per_ton > 1000.0 && cost.cif_per_ton < 1500.0);
        assert!(cost.total_cif.is_some());
        assert!((cost.total_fob.unwrap() + cost.total_frt.unwrap() - cost.total_cif.unwrap()).abs() < 1e-6);

        // 视图 B 检查
        let total_tons: f64 = r.orders.iter().filter_map(|o| o.tons).sum();
        assert!((total_tons - 3700.0).abs() < 1e-3, "总吨数 {} != 3700", total_tons);

        // 视图 C 检查: 至少存在一些指标体检结果
        assert!(!r.indicator_check.is_empty());
    }

    #[test]
    fn test_total_quantity_optional() {
        let coals = vec![
            coal_from_tuple("a", (2.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1100.0, 30.0)),
            coal_from_tuple("b", (1.5, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1200.0, 30.0)),
        ];
        let specs = vec![Spec::upper("S", 2.0), Spec::upper("A", 9.0)];
        let req = BlendRequest {
            coals,
            specs,
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: None,
        };
        let r = solve(&req);
        assert!(r.ok);
        // 没给总吨数 → tons 应为 None
        assert!(r.orders.iter().all(|o| o.tons.is_none()));
    }

    #[test]
    fn test_missing_indicator_skips_coal() {
        let mut bad = coal_from_tuple("缺胶质", (2.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1100.0, 30.0));
        bad.props.remove("Y");
        let good = coal_from_tuple("完整", (1.5, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1200.0, 30.0));

        let req = BlendRequest {
            coals: vec![bad, good],
            specs: vec![Spec::lower("Y", 14.0)],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: None,
        };
        let r = solve(&req);
        assert!(r.ok);
        assert!(r.warnings.iter().any(|w| w.contains("缺胶质")));
        assert_eq!(r.recipe.len(), 1);
        assert!(r.recipe.contains_key("完整"));
    }

    /// 验证 Direction::Upper 时即使 spec.min 有值也被忽略 (P0-1 修复).
    /// 让低硫煤便宜, 这样 LP 会优先选低硫. 如果 min=1.0 没被忽略, LP 会被迫
    /// 混入高硫煤以满足 S≥1.0, 让 S 卡在 1.0 附近; 忽略后 LP 自由选低硫=100%, S=0.5.
    #[test]
    fn test_direction_upper_ignores_min() {
        let coals = vec![
            coal_from_tuple("低硫便宜", (0.5, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1000.0, 30.0)),
            coal_from_tuple("高硫贵",   (3.0, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1500.0, 30.0)),
        ];
        let mut s = Spec::upper("S", 3.0);
        s.min = Some(1.0); // direction=Upper, 此值应被忽略
        let req = BlendRequest {
            coals,
            specs: vec![s],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: None,
        };
        let r = solve(&req);
        assert!(r.ok);
        let s_val = r.indicator_check.iter().find(|c| c.indicator == "S").unwrap().value;
        assert!(s_val < 0.6, "S = {} (应 ≈ 0.5 因为 min 被忽略, LP 选 100% 低硫便宜)", s_val);
    }

    /// 验证 slack 字段对无约束指标序列化为 null (P0-2 修复).
    #[test]
    fn test_unconstrained_indicator_slack_is_none() {
        let coals = vec![
            coal_from_tuple("a", (2.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1100.0, 30.0)),
        ];
        let req = BlendRequest {
            coals,
            specs: vec![Spec::upper("S", 3.0)], // 只对 S 加约束
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: None,
        };
        let r = solve(&req);
        assert!(r.ok);
        // S 有约束 → slack = Some(...)
        let s_check = r.indicator_check.iter().find(|c| c.indicator == "S").unwrap();
        assert!(s_check.slack.is_some());
        // V 无约束 → slack = None
        let v_check = r.indicator_check.iter().find(|c| c.indicator == "V").unwrap();
        assert!(v_check.slack.is_none(), "V 无约束应 slack=None, 实际 {:?}", v_check.slack);

        // 验证 JSON 序列化输出 null 而非 Infinity
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"slack\":null"), "应序列化为 null: {}", &json[..200.min(json.len())]);
    }

    #[test]
    fn test_binding_detection() {
        // 构造一个解必然把 S 顶到 2.5 的场景
        let coals = vec![
            coal_from_tuple("低硫贵", (1.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1500.0, 30.0)),
            coal_from_tuple("高硫便宜", (3.0, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1000.0, 30.0)),
        ];
        let req = BlendRequest {
            coals,
            specs: vec![Spec::upper("S", 2.5)],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: None,
        };
        let r = solve(&req);
        assert!(r.ok);
        let s_check = r.indicator_check.iter().find(|c| c.indicator == "S").unwrap();
        // 解会推到 S=2.5 (顶上限) 因为低硫煤贵
        assert!((s_check.value - 2.5).abs() < 0.01, "S = {}", s_check.value);
        assert!(s_check.binding, "S 约束应该 binding");
    }

    // ===== CSR 预测接入 (A 步) =====

    /// 生成满足 CSR = 30 + S + 0.5A + 0.8V + 0.3G + 0.6Y + 0.4M 的线性可拟合观测.
    /// 用 sin/cos 拉开 6 个特征的相关性, 保证 XᵀX 非奇异 (同 predict.rs 测试手法).
    fn perfect_csr_obs(n: usize) -> Vec<CsrObservation> {
        (0..n)
            .map(|i| {
                let t = i as f64;
                let s = 1.5 + (t * 0.7).sin().abs() * 1.2;
                let a = 6.0 + (t * 1.3).cos().abs() * 2.5;
                let v = 18.0 + (t * 0.5 + 1.0).sin().abs() * 8.0;
                let g = 75.0 + (t * 0.9).cos().abs() * 18.0;
                let y = 10.0 + (t * 1.7).sin().abs() * 9.0;
                let m = 8.0 + (t * 0.3 + 0.5).cos().abs() * 3.0;
                let csr = 30.0 + s + 0.5 * a + 0.8 * v + 0.3 * g + 0.6 * y + 0.4 * m;
                CsrObservation { s, a, v, g, y, m, csr_measured: csr }
            })
            .collect()
    }

    fn linbei() -> Coal {
        coal_from_tuple("临北", (2.0, 6.0, 22.0, 93.0, 17.0, 0.01, 70.0, 11.0, 1425.0, 25.0))
    }

    fn csr_value(r: &BlendResult) -> f64 {
        r.indicator_check.iter().find(|c| c.indicator == "CSR").unwrap().value
    }

    /// 提供观测时, 各煤 CSR 被回归预测覆盖. 单煤池 → 混合 CSR = 该煤预测值 ≈ 95.1
    /// (录入值 70 应被替换). 95.1 = 30 + 2 + 3 + 17.6 + 27.9 + 10.2 + 4.4.
    #[test]
    fn test_csr_prediction_overrides_recorded() {
        let req = BlendRequest {
            coals: vec![linbei()],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: Some(perfect_csr_obs(10)),
        };
        let r = solve(&req);
        assert!(r.ok, "{:?}", r.reason);
        let csr = csr_value(&r);
        assert!((csr - 95.1).abs() < 0.1, "预测 CSR 应 ≈95.1 (录入 70 被覆盖), 实际 {}", csr);
    }

    /// 不提供观测 → 行为不变, 保留录入 CSR=70.
    #[test]
    fn test_no_observations_keeps_recorded_csr() {
        let req = BlendRequest {
            coals: vec![linbei()],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: None,
        };
        let r = solve(&req);
        assert!(r.ok);
        assert!((csr_value(&r) - 70.0).abs() < 1e-6, "无观测应保留 CSR=70");
    }

    /// 样本不足 (<7) → 拟合失败, 回退录入 CSR 并加警告, 不静默吞掉.
    #[test]
    fn test_insufficient_observations_warns_and_falls_back() {
        let req = BlendRequest {
            coals: vec![linbei()],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: Some(perfect_csr_obs(5)),
        };
        let r = solve(&req);
        assert!(r.ok);
        assert!((csr_value(&r) - 70.0).abs() < 1e-6, "样本不足应回退 CSR=70");
        assert!(r.warnings.iter().any(|w| w.contains("CSR")), "应有 CSR 跳过警告: {:?}", r.warnings);
    }

    /// 生成线性关系极弱的观测: CSR 在 40/95 间高频交替, 平滑特征拟合不出 → R² 很低.
    fn noisy_csr_obs(n: usize) -> Vec<CsrObservation> {
        (0..n)
            .map(|i| {
                let t = i as f64;
                let s = 1.5 + (t * 0.7).sin().abs() * 1.2;
                let a = 6.0 + (t * 1.3).cos().abs() * 2.5;
                let v = 18.0 + (t * 0.5 + 1.0).sin().abs() * 8.0;
                let g = 75.0 + (t * 0.9).cos().abs() * 18.0;
                let y = 10.0 + (t * 1.7).sin().abs() * 9.0;
                let m = 8.0 + (t * 0.3 + 0.5).cos().abs() * 3.0;
                let csr = if i % 2 == 0 { 40.0 } else { 95.0 };
                CsrObservation { s, a, v, g, y, m, csr_measured: csr }
            })
            .collect()
    }

    /// 样本够但拟合质量差 (R² 低) → 不信任预测, 回退录入 CSR 并附 R² 警告.
    #[test]
    fn test_low_r2_falls_back_with_warning() {
        let req = BlendRequest {
            coals: vec![linbei()],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: Some(noisy_csr_obs(24)),
        };
        let r = solve(&req);
        assert!(r.ok);
        assert!((csr_value(&r) - 70.0).abs() < 1e-6, "低 R² 应回退 CSR=70, 实际 {}", csr_value(&r));
        assert!(r.warnings.iter().any(|w| w.contains("R²")), "应有 R² 不足警告: {:?}", r.warnings);
    }

    /// 拟合成功但某煤缺输入指标 → 该煤保留录入 CSR, 并逐煤点名警告 (不静默).
    #[test]
    fn test_coal_missing_input_keeps_csr_and_warns() {
        let mut no_g = linbei();
        no_g.name = "缺G".into();
        no_g.props.remove("G"); // 缺 G 输入 → 无法预测 CSR
        let req = BlendRequest {
            coals: vec![no_g],
            specs: vec![], // 无 CSR spec, 该煤不会被剔除
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: Some(perfect_csr_obs(10)),
        };
        let r = solve(&req);
        assert!(r.ok);
        assert!((csr_value(&r) - 70.0).abs() < 1e-6, "缺输入应保留录入 CSR=70");
        assert!(
            r.warnings.iter().any(|w| w.contains("缺G") && w.contains("CSR")),
            "应逐煤点名警告: {:?}",
            r.warnings
        );
    }

    /// 走真实 JSON 入口, 验证 serde 能反序列化 csr_observations 并生效.
    #[test]
    fn test_solve_json_accepts_csr_observations() {
        let req = BlendRequest {
            coals: vec![linbei()],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            csr_observations: Some(perfect_csr_obs(8)),
        };
        let out = solve_json(&serde_json::to_string(&req).unwrap());
        let r: BlendResult = serde_json::from_str(&out).unwrap();
        assert!(r.ok);
        assert!((csr_value(&r) - 95.1).abs() < 0.1, "JSON 入口预测 CSR 应 ≈95.1");
    }
}
