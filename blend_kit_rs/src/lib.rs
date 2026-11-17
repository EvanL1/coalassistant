// 单元测试 (#[cfg(test)] 模块) 里的浮点相等断言是测试意图, 生产代码仍 deny float_cmp.
#![cfg_attr(test, allow(clippy::float_cmp))]

//! 豆哥配煤 - 配煤优化核心算法.
//!
//! 数据流程:
//!   业务侧 4 张表 (化验/合同/煤价/物流) → 归一为 COALS + SPECS
//!   → Clarabel 最低成本配方 → 逐指标按合同判定规则体检
//!   → 3 个业务视图 (成本结构 / 实物订单 / 指标体检)
pub mod model;
pub mod optimizer;
mod penalty;
pub mod predict;
pub mod price_factor;
pub mod pricing;
mod quality;
mod rank_interaction;
mod robustness;
pub mod seed;
mod shovel;
pub use predict::{CsrObservation, CsrPredictor};
pub use rank_interaction::{coal_rank, fit_k, rank_moments, KFit, MIN_CALIBRATION_SAMPLES};
pub use seed::{CoalMaster, CoalMasterEntry, Confidence, DefaultContract, MasterStatus};

pub use model::*;
pub use optimizer::solve;
pub use pricing::estimate_json;

/// 返回编译进核心 crate 的 Master JSON 原文.
///
/// HTTP 服务端与命令行都必须通过这个入口读取，避免维护静态副本.
pub fn master_json() -> &'static str {
    include_str!("../data/coal_master.json")
}

/// JSON in/out 入口 (前端通过此函数调用).
pub fn solve_json(input_json: &str) -> String {
    let result = match serde_json::from_str::<BlendRequest>(input_json) {
        Ok(req) => solve(&req),
        Err(e) => BlendResult::infeasible(&format!("JSON 解析失败: {}", e), Vec::new()),
    };
    serde_json::to_string(&result)
        .unwrap_or_else(|_| r#"{"ok":false,"reason":"序列化失败"}"#.to_string())
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
        purchase_terms: None,
        coal_type: None,
        form: None,
        region: None,
        province: None,
        city: None,
        county: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用你昨晚验证过的临北数据.
    #[test]
    fn test_linbei_tuple() {
        let linbei = coal_from_tuple(
            "临北",
            (2.0, 6.0, 22.0, 93.0, 17.0, 0.01, 70.0, 11.0, 1425.0, 25.0),
        );
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
            coal_from_tuple(
                "铁新",
                (3.2, 6.5, 21.0, 92.0, 14.0, 0.10, 62.0, 7.5, 1260.0, 30.0),
            ),
            coal_from_tuple(
                "临北",
                (2.0, 6.0, 22.0, 93.0, 16.0, 0.10, 66.0, 7.0, 1250.0, 30.0),
            ),
            coal_from_tuple(
                "安益",
                (3.4, 6.8, 18.0, 75.0, 10.0, 0.18, 60.0, 8.0, 1120.0, 30.0),
            ),
            coal_from_tuple(
                "筛精",
                (3.9, 9.5, 25.0, 100.0, 22.0, 0.08, 65.0, 9.5, 970.0, 30.0),
            ),
            coal_from_tuple(
                "孟子峪",
                (2.0, 9.5, 18.0, 75.0, 10.0, 0.18, 58.0, 7.8, 1034.0, 30.0),
            ),
            coal_from_tuple(
                "大佛寺",
                (3.0, 8.5, 18.0, 75.0, 10.0, 0.16, 59.0, 8.0, 1120.0, 30.0),
            ),
            coal_from_tuple(
                "神州",
                (2.6, 10.0, 17.0, 65.0, 8.0, 0.20, 55.0, 7.2, 1110.0, 30.0),
            ),
            coal_from_tuple(
                "豹子沟",
                (3.8, 11.0, 24.0, 92.0, 20.0, 0.12, 64.0, 8.5, 1250.0, 30.0),
            ),
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
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok, "expected feasible: {:?}", r.reason);

        let cost = r.cost.unwrap();
        assert!(cost.cif_per_ton > 1000.0 && cost.cif_per_ton < 1500.0);
        assert!(cost.total_cif.is_some());
        assert!(
            (cost.total_fob.unwrap() + cost.total_frt.unwrap() - cost.total_cif.unwrap()).abs()
                < 1e-6
        );

        // 视图 B 检查
        let total_tons: f64 = r.orders.iter().filter_map(|o| o.tons).sum();
        assert!(
            (total_tons - 3700.0).abs() < 1e-3,
            "总吨数 {} != 3700",
            total_tons
        );

        // 视图 C 检查: 至少存在一些指标体检结果
        assert!(!r.indicator_check.is_empty());
    }

    #[test]
    fn test_total_quantity_optional() {
        let coals = vec![
            coal_from_tuple(
                "a",
                (2.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1100.0, 30.0),
            ),
            coal_from_tuple(
                "b",
                (1.5, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1200.0, 30.0),
            ),
        ];
        let specs = vec![Spec::upper("S", 2.0), Spec::upper("A", 9.0)];
        let req = BlendRequest {
            coals,
            specs,
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok);
        // 没给总吨数 → tons 应为 None
        assert!(r.orders.iter().all(|o| o.tons.is_none()));
    }

    #[test]
    fn test_missing_indicator_skips_coal() {
        let mut bad = coal_from_tuple(
            "缺胶质",
            (2.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1100.0, 30.0),
        );
        bad.props.remove("Y");
        let good = coal_from_tuple(
            "完整",
            (1.5, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1200.0, 30.0),
        );

        let req = BlendRequest {
            coals: vec![bad, good],
            specs: vec![Spec::lower("Y", 14.0)],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
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
            coal_from_tuple(
                "低硫便宜",
                (0.5, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1000.0, 30.0),
            ),
            coal_from_tuple(
                "高硫贵",
                (3.0, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1500.0, 30.0),
            ),
        ];
        let mut s = Spec::upper("S", 3.0);
        s.min = Some(1.0); // direction=Upper, 此值应被忽略
        let req = BlendRequest {
            coals,
            specs: vec![s],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok);
        let s_val = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "S")
            .unwrap()
            .value;
        assert!(
            s_val < 0.6,
            "S = {} (应 ≈ 0.5 因为 min 被忽略, LP 选 100% 低硫便宜)",
            s_val
        );
    }

    /// Upper 带一个比上限还大的残留 min 时照常求解 (P0-1 的彻底版): 校验不得拒绝,
    /// 体检也不再回显被忽略的那一侧.
    #[test]
    fn test_upper_ignores_stale_larger_min() {
        let coals = vec![coal_from_tuple(
            "单煤",
            (0.5, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1000.0, 30.0),
        )];
        let mut s = Spec::upper("S", 3.0);
        s.min = Some(99.0); // 残留值, 比上限还大; Upper 方向应被忽略
        let req = BlendRequest {
            coals,
            specs: vec![s],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok, "{:?}", r.reason);
        let check = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "S")
            .expect("应有 S 体检行");
        assert_eq!(check.min, None, "被忽略的 min 不回显");
        assert_eq!(check.max, Some(3.0));
    }

    /// Lower 忽略 max 的对称用例.
    #[test]
    fn test_lower_ignores_stale_smaller_max() {
        let coals = vec![coal_from_tuple(
            "单煤",
            (0.5, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1000.0, 30.0),
        )];
        let mut g = Spec::lower("G", 80.0);
        g.max = Some(10.0); // 残留值, 比下限还小; Lower 方向应被忽略
        let req = BlendRequest {
            coals,
            specs: vec![g],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok, "{:?}", r.reason);
        let check = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "G")
            .expect("应有 G 体检行");
        assert_eq!(check.max, None, "被忽略的 max 不回显");
        assert_eq!(check.min, Some(80.0));
    }
    /// 验证 slack 字段对无约束指标序列化为 null (P0-2 修复).
    #[test]
    fn test_unconstrained_indicator_slack_is_none() {
        let coals = vec![coal_from_tuple(
            "a",
            (2.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1100.0, 30.0),
        )];
        let req = BlendRequest {
            coals,
            specs: vec![Spec::upper("S", 3.0)], // 只对 S 加约束
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok);
        // S 有约束 → slack = Some(...)
        let s_check = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "S")
            .unwrap();
        assert!(s_check.slack.is_some());
        // V 无约束 → slack = None
        let v_check = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "V")
            .unwrap();
        assert!(
            v_check.slack.is_none(),
            "V 无约束应 slack=None, 实际 {:?}",
            v_check.slack
        );

        // 验证 JSON 序列化输出 null 而非 Infinity
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            json.contains("\"slack\":null"),
            "应序列化为 null: {}",
            &json[..200.min(json.len())]
        );
    }

    #[test]
    fn test_binding_detection() {
        // 构造一个解必然把 S 顶到 2.5 的场景
        let coals = vec![
            coal_from_tuple(
                "低硫贵",
                (1.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1500.0, 30.0),
            ),
            coal_from_tuple(
                "高硫便宜",
                (3.0, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1000.0, 30.0),
            ),
        ];
        let req = BlendRequest {
            coals,
            specs: vec![Spec::upper("S", 2.5)],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok);
        let s_check = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "S")
            .unwrap();
        // 解会推到 S=2.5 (顶上限) 因为低硫煤贵
        assert!((s_check.value - 2.5).abs() < 0.01, "S = {}", s_check.value);
        assert!(s_check.binding, "S 约束应该 binding");
    }

    // ===== 录入的 CSR =====

    fn linbei() -> Coal {
        coal_from_tuple(
            "临北",
            (2.0, 6.0, 22.0, 93.0, 17.0, 0.01, 70.0, 11.0, 1425.0, 25.0),
        )
    }

    fn csr_value(r: &BlendResult) -> f64 {
        r.indicator_check
            .iter()
            .find(|c| c.indicator == "CSR")
            .unwrap()
            .value
    }

    /// 体检里的 CSR 是各煤录入值按配比加权: 单煤池就是录入的 70.
    #[test]
    fn test_no_observations_keeps_recorded_csr() {
        let req = BlendRequest {
            coals: vec![linbei()],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok);
        assert!((csr_value(&r) - 70.0).abs() < 1e-6, "无观测应保留 CSR=70");
    }

    // ===== 安全余量 margin (调研 2026-07-04 阶段 0) =====

    /// margin 收紧上限: S max 2.5 + margin 0.2 → LP 内部按 2.3 求解.
    /// 展示层 (indicator_check.max) 仍报合同原值 2.5.
    #[test]
    fn test_spec_margin_tightens_upper() {
        let coals = vec![
            coal_from_tuple(
                "低硫贵",
                (1.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1500.0, 30.0),
            ),
            coal_from_tuple(
                "高硫便宜",
                (3.0, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1000.0, 30.0),
            ),
        ];
        let mut s = Spec::upper("S", 2.5);
        s.margin = Some(0.2);
        let req = BlendRequest {
            coals,
            specs: vec![s],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok, "{:?}", r.reason);
        let s_check = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "S")
            .unwrap();
        assert!(
            (s_check.value - 2.3).abs() < 0.01,
            "S 应贴收紧后上限 2.3, 实际 {}",
            s_check.value
        );
        assert_eq!(s_check.max, Some(2.5), "展示层应保留合同原上限");
    }

    /// margin 抬高下限: G min 80 + margin 3 → LP 内部按 83 求解.
    #[test]
    fn test_spec_margin_tightens_lower() {
        let coals = vec![
            coal_from_tuple(
                "高粘贵",
                (2.0, 8.0, 22.0, 95.0, 15.0, 0.10, 65.0, 8.0, 1500.0, 30.0),
            ),
            coal_from_tuple(
                "低粘便宜",
                (2.0, 8.5, 23.0, 60.0, 16.0, 0.10, 66.0, 7.5, 1000.0, 30.0),
            ),
        ];
        let mut g = Spec::lower("G", 80.0);
        g.margin = Some(3.0);
        let req = BlendRequest {
            coals,
            specs: vec![g],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok, "{:?}", r.reason);
        let g_check = r
            .indicator_check
            .iter()
            .find(|c| c.indicator == "G")
            .unwrap();
        assert!(
            (g_check.value - 83.0).abs() < 0.01,
            "G 应贴抬高后下限 83, 实际 {}",
            g_check.value
        );
        assert_eq!(g_check.min, Some(80.0), "展示层应保留合同原下限");
    }

    /// 旧版请求 JSON (无 margin 字段) 应正常解析且行为不变.
    #[test]
    fn test_spec_json_without_margin_parses() {
        let json = r#"{"indicator":"S","direction":"Upper","min":null,"max":2.5,"enabled":true}"#;
        let s: Spec = serde_json::from_str(json).unwrap();
        assert!(s.margin.is_none());
    }

    // ===== 岩相: 八项里的一个普通指标 =====

    /// 岩相按八项中的 petro 标量加权, 标为线性估算 (未验证)。
    #[test]
    fn test_petro_no_data_is_unverified() {
        let coals = vec![coal_from_tuple(
            "无煤岩",
            (2.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1100.0, 30.0),
        )];
        let mut petro_spec = Spec::upper("petro", 0.15);
        petro_spec.enforcement = Enforcement::Advisory;
        let req = BlendRequest {
            coals,
            specs: vec![petro_spec],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(r.ok);
        let check = r
            .indicator_check
            .iter()
            .find(|check| check.indicator == "petro")
            .expect("应保留岩相指标状态");
        assert_eq!(check.method, EvaluationMethod::ProvisionalLinear);
        assert_eq!(check.status, EvaluationStatus::Unverified);
        assert!(r.warnings.is_empty());
    }

    /// 负 margin 会反向放宽约束产出违约配比 → 必须拒绝请求.
    #[test]
    fn test_negative_margin_rejected() {
        let coals = vec![
            coal_from_tuple(
                "低硫贵",
                (1.0, 8.0, 22.0, 90.0, 15.0, 0.10, 65.0, 8.0, 1500.0, 30.0),
            ),
            coal_from_tuple(
                "高硫便宜",
                (3.0, 8.5, 23.0, 92.0, 16.0, 0.10, 66.0, 7.5, 1000.0, 30.0),
            ),
        ];
        let mut s = Spec::upper("S", 2.5);
        s.margin = Some(-0.5); // 恶意/手滑输入: 若不钳制, LP 按 3.0 放行
        let req = BlendRequest {
            coals,
            specs: vec![s],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let r = solve(&req);
        assert!(!r.ok);
        assert!(
            r.reason
                .as_deref()
                .is_some_and(|reason| reason.contains("安全余量")),
            "应明确指出安全余量非法: {:?}",
            r.reason
        );
    }

    /// 基础 JSON 求解不读取训练样本；旧调用方残留的字段会被兼容忽略。
    #[test]
    fn test_solve_json_ignores_training_fields() {
        let req = BlendRequest {
            coals: vec![linbei()],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let mut input = serde_json::to_value(&req).unwrap();
        input["csr_observations"] = serde_json::json!([
            {"s": 1.5, "a": 8.5, "v": 24.7, "g": 93.0, "y": 10.0, "m": 10.6, "csr_measured": 95.0}
        ]);
        input["model_policy"] = serde_json::json!({"min_csr_samples": 8, "max_csr_cv_mae": 1.0});
        let out = solve_json(&input.to_string());
        let r: BlendResult = serde_json::from_str(&out).unwrap();
        assert!(r.ok);
        assert!(
            (csr_value(&r) - 70.0).abs() < 1e-6,
            "基础 JSON 求解应保留录入 CSR"
        );
    }

    /// 不含 penalty / purchase_terms 的旧请求: 求解结果不变, 新字段取中性值.
    #[test]
    fn test_legacy_request_keeps_neutral_penalty_fields() {
        let coals = vec![
            coal_from_tuple(
                "甲",
                (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 50.0),
            ),
            coal_from_tuple(
                "乙",
                (0.8, 8.0, 26.0, 90.0, 18.0, 0.10, 66.0, 8.0, 1100.0, 50.0),
            ),
        ];
        let request = BlendRequest {
            coals,
            specs: vec![Spec::upper("A", 10.0)],
            total_quantity: Some(1000.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "存量请求应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert_eq!(cost.penalty_per_ton, 0.0, "无计价条款时卖出侧扣款应为 0");
        assert_eq!(
            cost.purchase_adjust_per_ton, 0.0,
            "无采购条款时买入侧修正应为 0"
        );
        assert!(
            (cost.net_per_ton - cost.cif_per_ton).abs() < 1e-9,
            "净成本应等于到厂价"
        );
        assert_eq!(cost.total_purchase_adjust, Some(0.0));
        assert_eq!(cost.total_penalty, Some(0.0));
        assert_eq!(cost.total_net, cost.total_cif, "净成本合计应等于到厂价合计");

        // cif_eff_per_ton 是按煤而非常量的字段, 逐煤核对具体值而非只查有限.
        // 甲 = 1000 + 50 = 1050, 乙 = 1100 + 50 = 1150.
        assert!(!result.orders.is_empty(), "应至少有一条订单");
        for order in &result.orders {
            let expected_cif = match order.coal.as_str() {
                "甲" => 1050.0,
                "乙" => 1150.0,
                other => panic!("意外煤种: {other}"),
            };
            assert_eq!(
                order.cif_eff_per_ton, expected_cif,
                "{} 的 cif_eff_per_ton 应等于其到厂价",
                order.coal
            );
        }
        for check in &result.indicator_check {
            assert_eq!(check.penalty_per_ton, None, "非计价指标扣款应为 None");
        }
    }

    /// 老 JSON(不含 penalty/purchase_terms)必须仍能解析并求解.
    /// 这条测试守的是 solve_json 的字符串契约, 上一条测的是 Rust 结构体路径.
    #[test]
    fn test_solve_json_accepts_legacy_payload() {
        let payload = r#"{
            "coals": [
                {"name":"甲","props":{"S":1.0,"A":9.0,"V":24.0,"G":88.0,"Y":16.0,"petro":0.10,"CSR":65.0,"M":8.0},"fob":1000.0,"frt":50.0},
                {"name":"乙","props":{"S":0.8,"A":8.0,"V":26.0,"G":90.0,"Y":18.0,"petro":0.10,"CSR":66.0,"M":8.0},"fob":1100.0,"frt":50.0}
            ],
            "specs": [{"indicator":"A","direction":"Upper","min":null,"max":10.0,"enabled":true}],
            "total_quantity": 1000.0,
            "truncate_decimal": false
        }"#;
        let output = solve_json(payload);
        let result: BlendResult = serde_json::from_str(&output).expect("结果应可反序列化");
        assert!(result.ok, "老 JSON 应可求解: {:?}", result.reason);
    }

    /// 复现 BLOCKING 1: PostgreSQL 中存量 BlendResult 记录的 cost/orders
    /// 缺扣款字段 (上线前写入); 没有 #[serde(default)] 会导致反序列化直接报错.
    #[test]
    fn test_legacy_blend_result_json_deserializes() {
        let payload = r#"{
            "ok": true,
            "reason": null,
            "recipe": {"甲": 1.0},
            "cost": {
                "fob_per_ton": 1000.0,
                "frt_per_ton": 50.0,
                "cif_per_ton": 1050.0,
                "total_fob": 1000000.0,
                "total_frt": 50000.0,
                "total_cif": 1050000.0
            },
            "orders": [
                {
                    "coal": "甲",
                    "ratio": 1.0,
                    "tons": 1000.0,
                    "fob_amount": 1000000.0,
                    "frt_amount": 50000.0,
                    "cif_amount": 1050000.0
                }
            ],
            "indicator_check": [],
            "warnings": []
        }"#;
        let result: BlendResult =
            serde_json::from_str(payload).expect("缺扣款字段的存量记录应仍可反序列化");
        let cost = result.cost.expect("应有成本");
        assert_eq!(cost.purchase_adjust_per_ton, 0.0);
        assert_eq!(cost.penalty_per_ton, 0.0);
        assert_eq!(cost.net_per_ton, 0.0);
        assert_eq!(result.orders[0].cif_eff_per_ton, 0.0);
        assert_eq!(result.orders[0].cif_eff_amount, None);
    }

    fn priced_upper_spec(
        indicator: &str,
        maximum: f64,
        tiers: Vec<PenaltyTier>,
        reject: f64,
    ) -> Spec {
        Spec {
            indicator: indicator.into(),
            direction: Direction::Upper,
            min: None,
            max: Some(maximum),
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Priced,
            penalty: Some(Penalty { tiers, reject }),
        }
    }

    fn priced_lower_spec(
        indicator: &str,
        minimum: f64,
        tiers: Vec<PenaltyTier>,
        reject: f64,
    ) -> Spec {
        Spec {
            indicator: indicator.into(),
            direction: Direction::Lower,
            min: Some(minimum),
            max: None,
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Priced,
            penalty: Some(Penalty { tiers, reject }),
        }
    }

    /// 偏离 1.5 跨两档: 一档(宽 1.0, 10 元)填满 + 二档(30 元)承接 0.5 ⇒ 10 + 15 = 25 元/吨.
    /// 不需要次序约束, 因为二档更贵, 最小化目标自行按序填档.
    #[test]
    fn test_priced_penalty_fills_cheap_tier_first() {
        let request = BlendRequest {
            coals: vec![coal_from_tuple(
                "甲",
                (1.0, 11.5, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            )],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![
                    PenaltyTier {
                        width: Some(1.0),
                        rate: 10.0,
                    },
                    PenaltyTier {
                        width: None,
                        rate: 30.0,
                    },
                ],
                13.0,
            )],
            total_quantity: Some(100.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "计价超界不应判不可行: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.penalty_per_ton - 25.0).abs() < 1e-3,
            "扣款应为 25 元/吨, 实得 {}",
            cost.penalty_per_ton
        );
        assert!(
            (cost.net_per_ton - 1025.0).abs() < 1e-3,
            "净成本应为 1025, 实得 {}",
            cost.net_per_ton
        );
        // 内点法收敛到 ~1e-6, 总额不能用 assert_eq! 比浮点.
        assert!(
            cost.total_penalty
                .is_some_and(|value| (value - 2500.0).abs() < 1e-2),
            "扣款总额应为 2500, 实得 {:?}",
            cost.total_penalty
        );

        let ash = result
            .indicator_check
            .iter()
            .find(|check| check.indicator == "A")
            .expect("应有灰分体检");
        assert!(
            ash.penalty_per_ton
                .is_some_and(|value| (value - 25.0).abs() < 1e-3),
            "灰分应带扣款额"
        );
        assert_eq!(
            ash.status,
            EvaluationStatus::TolerancePass,
            "计价带内超合同界应判 TolerancePass 而非 Fail"
        );
        assert_ne!(result.quality_status, QualityStatus::NeedsReview);
    }

    /// 越过拒收线仍是硬不可行 —— 这是防止求解器算出商业自杀方案的唯一机制.
    #[test]
    fn test_priced_penalty_reject_line_is_hard() {
        let request = BlendRequest {
            coals: vec![coal_from_tuple(
                "甲",
                (1.0, 13.5, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            )],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 10.0,
                }],
                13.0,
            )],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(!result.ok, "越过拒收线应不可行");
    }

    /// 便宜脏煤 vs 贵干净煤: 省 80 元、扣 64 元 ⇒ 应选脏煤.
    /// 硬约束模型会把脏煤判为不可行, 白丢 16 元/吨.
    #[test]
    fn test_priced_penalty_prefers_cheaper_off_spec_coal() {
        let request = BlendRequest {
            coals: vec![
                coal_from_tuple(
                    "干净",
                    (1.0, 10.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 2280.0, 0.0),
                ),
                coal_from_tuple(
                    "便宜",
                    (1.0, 10.8, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 2200.0, 0.0),
                ),
            ],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 80.0,
                }],
                11.5,
            )],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        let cheap = result.recipe.get("便宜").copied().unwrap_or(0.0);
        assert!(cheap > 0.999, "应全选便宜煤, 实得配比 {cheap}");

        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.penalty_per_ton - 64.0).abs() < 1e-2,
            "扣款应为 64 元/吨, 实得 {}",
            cost.penalty_per_ton
        );
        assert!(
            (cost.net_per_ton - 2264.0).abs() < 1e-2,
            "净成本应为 2264 (优于干净煤的 2280), 实得 {}",
            cost.net_per_ton
        );
    }

    /// Lower 向的吸收行: 合同 G≥85, 单煤 G=83, 欠 2 点 × 5 元/吨·点 ⇒ 10 元/吨.
    /// 这正是"宁可少 1 点粘结吃 5 元扣款, 也不花几十元换贵煤"的业务场景.
    #[test]
    fn test_priced_penalty_lower_direction_absorbs_shortfall() {
        let request = BlendRequest {
            coals: vec![coal_from_tuple(
                "甲",
                (1.0, 9.0, 24.0, 83.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            )],
            specs: vec![priced_lower_spec(
                "G",
                85.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 5.0,
                }],
                80.0,
            )],
            total_quantity: Some(100.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "计价欠界不应判不可行: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.penalty_per_ton - 10.0).abs() < 1e-3,
            "扣款应为 10 元/吨, 实得 {}",
            cost.penalty_per_ton
        );
        assert!(
            (cost.net_per_ton - 1010.0).abs() < 1e-3,
            "净成本应为 1010, 实得 {}",
            cost.net_per_ton
        );
        assert!(
            cost.total_penalty
                .is_some_and(|value| (value - 1000.0).abs() < 1e-2),
            "扣款总额应为 1000, 实得 {:?}",
            cost.total_penalty
        );

        let cohesion = result
            .indicator_check
            .iter()
            .find(|check| check.indicator == "G")
            .expect("应有粘结体检");
        assert!(
            cohesion
                .penalty_per_ton
                .is_some_and(|value| (value - 10.0).abs() < 1e-3),
            "粘结应带扣款额"
        );
        assert_eq!(
            cohesion.status,
            EvaluationStatus::TolerancePass,
            "计价带内低于合同界应判 TolerancePass 而非 Fail"
        );
    }

    /// Lower 向的悬崖行: 拒收线是下限, 低于它硬不可行.
    /// 与 Upper 向共用代码但符号相反, 单独钉住防止 effective_lower 的 margin 方向写反.
    #[test]
    fn test_priced_penalty_lower_reject_line_is_hard() {
        let request = BlendRequest {
            coals: vec![coal_from_tuple(
                "甲",
                (1.0, 9.0, 24.0, 79.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            )],
            specs: vec![priced_lower_spec(
                "G",
                85.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 5.0,
                }],
                80.0,
            )],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(!result.ok, "低于拒收线应不可行");
    }

    /// Priced 的拒收线是 LP 硬行, 缺输入就建不出约束 —— 必须和 Hard 一样剔煤并留 warning.
    /// 回归 Task 4 引入的不一致: 此前 Priced 不参与剔煤, 缺 A 的煤会把整次求解拖成
    /// "约束冲突, LP 不可行" 且不指名罪魁, 而同样煤池在 Hard 下能剔煤后正常求解.
    #[test]
    fn test_priced_spec_culls_coal_missing_indicator() {
        let mut incomplete = coal_from_tuple(
            "缺灰",
            (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 900.0, 0.0),
        );
        incomplete.props.remove("A");
        let complete = coal_from_tuple(
            "完整",
            (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
        );
        let request = BlendRequest {
            coals: vec![incomplete, complete],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 80.0,
                }],
                13.0,
            )],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "剔掉缺输入的煤后应可解: {:?}", result.reason);
        assert!(
            !result.recipe.contains_key("缺灰"),
            "缺灰分输入的煤不应进配方"
        );
        assert!(result.recipe.contains_key("完整"), "完整煤应进配方");
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("剔除") && warning.contains("缺灰")),
            "应有指名缺灰的剔除 warning, 实得 {:?}",
            result.warnings
        );
    }

    /// 两条计价约束同时生效: 档位列块必须互不重叠且各自读回自己的扣款.
    ///
    /// 只有一个块时 offset 恒等于 count, 偏移算错也看不出来; 第二个块才真正检验
    /// 块间偏移. 两块故意取不同档数 (A 两档宽 2 列, G 一档宽 1 列), 等宽会掩盖差一错误.
    /// 布局: 列 0 = 配比, 列 1~2 = A 的两档, 列 3 = G 的单档.
    #[test]
    fn test_two_priced_specs_keep_separate_tier_blocks() {
        let request = BlendRequest {
            coals: vec![coal_from_tuple(
                "甲",
                (1.0, 11.5, 24.0, 83.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            )],
            specs: vec![
                // 灰分超 1.5: 一档(宽 1.0, 10 元)满 + 二档(30 元)承接 0.5 ⇒ 25 元/吨.
                priced_upper_spec(
                    "A",
                    10.0,
                    vec![
                        PenaltyTier {
                            width: Some(1.0),
                            rate: 10.0,
                        },
                        PenaltyTier {
                            width: None,
                            rate: 30.0,
                        },
                    ],
                    13.0,
                ),
                // 粘结欠 2 点 × 5 元/吨·点 ⇒ 10 元/吨.
                priced_lower_spec(
                    "G",
                    85.0,
                    vec![PenaltyTier {
                        width: None,
                        rate: 5.0,
                    }],
                    80.0,
                ),
            ],
            total_quantity: Some(100.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "两条计价约束应可解: {:?}", result.reason);

        let penalty_of = |indicator: &str| -> f64 {
            result
                .indicator_check
                .iter()
                .find(|check| check.indicator == indicator)
                .unwrap_or_else(|| panic!("应有 {indicator} 体检"))
                .penalty_per_ton
                .unwrap_or_else(|| panic!("{indicator} 应带扣款额"))
        };
        // 各记各的: 25 / 10, 既不串块也不是合计 35.
        assert!(
            (penalty_of("A") - 25.0).abs() < 1e-3,
            "灰分扣款应为 25 元/吨, 实得 {}",
            penalty_of("A")
        );
        assert!(
            (penalty_of("G") - 10.0).abs() < 1e-3,
            "粘结扣款应为 10 元/吨, 实得 {}",
            penalty_of("G")
        );

        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.penalty_per_ton - 35.0).abs() < 1e-3,
            "合计扣款应为 35 元/吨, 实得 {}",
            cost.penalty_per_ton
        );
        assert!(
            (cost.net_per_ton - (cost.cif_per_ton + 35.0)).abs() < 1e-3,
            "净成本应为 cif + 35, 实得 {} (cif {})",
            cost.net_per_ton,
            cost.cif_per_ton
        );
        assert!(
            (cost.net_per_ton - 1035.0).abs() < 1e-3,
            "净成本应为 1035, 实得 {}",
            cost.net_per_ton
        );
    }

    /// margin 只收紧拒收线, 绝不移动合同界 —— 合同界是结算用的精确合同数字,
    /// 移动它会让每吨凭空多扣 margin × rate, 且不会报错, 只是安静地算贵.
    /// 无 margin 与有 margin 两次求解的扣款必须逐分相等.
    #[test]
    fn test_priced_margin_does_not_shift_contract_limit() {
        let penalty_with_margin = |margin: Option<f64>| -> f64 {
            let mut spec = priced_upper_spec(
                "A",
                10.0,
                vec![
                    PenaltyTier {
                        width: Some(1.0),
                        rate: 10.0,
                    },
                    PenaltyTier {
                        width: None,
                        rate: 30.0,
                    },
                ],
                13.0,
            );
            spec.margin = margin;
            let request = BlendRequest {
                coals: vec![coal_from_tuple(
                    "甲",
                    (1.0, 11.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
                )],
                specs: vec![spec],
                total_quantity: None,
                truncate_decimal: false,
                fixed_ratios: None,
                rank_interaction: None,
                price_factor: None,
                price_robustness: None,
                shovel_tons: None,
                min_ratios: Default::default(),
            };
            let result = solve(&request);
            assert!(result.ok, "margin={margin:?} 时应可解: {:?}", result.reason);
            result.cost.expect("应有成本").penalty_per_ton
        };

        // 偏离 1.0: 恰好填满一档(宽 1.0, 10 元) ⇒ 10 元/吨.
        let without_margin = penalty_with_margin(None);
        assert!(
            (without_margin - 10.0).abs() < 1e-3,
            "无 margin 扣款应为 10 元/吨, 实得 {without_margin}"
        );

        // 若 margin 误加到合同界上: 界变 9.5, 偏离 1.5 ⇒ 10 + 0.5×30 = 25 元/吨.
        // 取 11.0 而非 11.5, 是为了让变异态仍可解, 从而由下面的相等断言抓住差异,
        // 而不是靠上面的 result.ok 断言间接抓住.
        let with_margin = penalty_with_margin(Some(0.5));
        assert!(
            (without_margin - with_margin).abs() < 1e-6,
            "margin 不得改变合同界扣款: 无 margin {without_margin}, 有 margin {with_margin}"
        );
    }

    /// margin 必须真的收紧拒收线 —— 否则"margin 不影响扣款"可以靠彻底废掉
    /// margin 来伪造通过. 灰分 11.25 落在 [拒收线 − margin, 拒收线] 区间内:
    /// 无 margin 时未越 12.0 拒收线应可解, margin 1.0 把线收到 11.0 后必须不可行.
    #[test]
    fn test_priced_margin_tightens_reject_line() {
        let solves_with_margin = |margin: Option<f64>| -> bool {
            let mut spec = priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 10.0,
                }],
                12.0,
            );
            spec.margin = margin;
            let request = BlendRequest {
                coals: vec![coal_from_tuple(
                    "甲",
                    (1.0, 11.25, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
                )],
                specs: vec![spec],
                total_quantity: None,
                truncate_decimal: false,
                fixed_ratios: None,
                rank_interaction: None,
                price_factor: None,
                price_robustness: None,
                shovel_tons: None,
                min_ratios: Default::default(),
            };
            solve(&request).ok
        };

        assert!(
            solves_with_margin(None),
            "灰分 11.25 未越 12.0 拒收线, 无 margin 时应可解"
        );
        assert!(
            !solves_with_margin(Some(1.0)),
            "margin 1.0 把拒收线收紧到 11.0, 灰分 11.25 应判不可行"
        );
    }

    /// 回归: 计价解曾被可行性复算误判为不可行.
    ///
    /// 机理: 吸收行在每个计价最优解处按构造都是紧的 (目标函数把 Σd 压到恰好等于偏离量);
    /// 配比列经归一化重投影后残差塌到 ~1e-13, 但档位列不归一化, 残差保持在 Clarabel
    /// 的原始收敛量级 ~1e-8..1e-7. 复算原本用绝对 1e-8, 于是把正确解判成越界 ——
    /// 本例 ash=12.5/reject=13.0 曾解出 d=2.499999961849723, 吸收行残差 3.8e-8 > 1e-8,
    /// 退化成 "约束冲突, LP 不可行". 相对判据 FEASIBILITY_TOLERANCE 修复.
    #[test]
    fn test_priced_solution_survives_feasibility_recheck() {
        let request = BlendRequest {
            coals: vec![coal_from_tuple(
                "甲",
                (1.0, 12.5, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            )],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 10.0,
                }],
                13.0,
            )],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "计价解不应被复算误判不可行: {:?}", result.reason);

        // 偏离 2.5 × 10 元 ⇒ 25 元/吨.
        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.penalty_per_ton - 25.0).abs() < 1e-3,
            "扣款应为 25 元/吨, 实得 {}",
            cost.penalty_per_ton
        );
    }

    /// 回归: 偏离恰为 0 时档位变量的最优解是 0, 内点法从下方逼近落在 -1e-8 量级,
    /// 非负性门限 (同样因档位列不归一化) 曾据此判不可行. 煤正好压合同上限是常见情形.
    #[test]
    fn test_priced_zero_deviation_survives_nonnegativity_gate() {
        let request = BlendRequest {
            coals: vec![coal_from_tuple(
                "甲",
                (1.0, 10.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
            )],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 10.0,
                }],
                14.0,
            )],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "零偏离计价解应可行: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert!(
            cost.penalty_per_ton.abs() < 1e-6,
            "正好压合同上限不应有扣款, 实得 {}",
            cost.penalty_per_ton
        );
    }

    /// 放宽复算容限后拒收线仍须是硬墙: 超线 0.01% (化验一个刻度) 必须判不可行.
    ///
    /// 放行量是 `容限 × (1 + 量级)`, **不是容限本身** —— 这里曾写成"约 1e-7 指标
    /// 单位, 细 5 个数量级", 把常量当成了放行量, 漏掉的正是这一轮修复的那个因子.
    /// 本例拒收线 12.0, 放行量约 1.3e-6, 比化验 0.01 细约 4 个数量级.
    /// 实测的放行量上限见 `quality::ACCEPT_TOLERANCE` 那张表 (本 crate 唯一真源).
    ///
    /// 实测超线 1e-9 即被拦下, 正好压线则仍可行.
    #[test]
    fn test_reject_line_still_blocks_one_assay_increment() {
        let solves = |ash: f64| -> bool {
            let request = BlendRequest {
                coals: vec![coal_from_tuple(
                    "甲",
                    (1.0, ash, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
                )],
                specs: vec![priced_upper_spec(
                    "A",
                    10.0,
                    vec![PenaltyTier {
                        width: None,
                        rate: 10.0,
                    }],
                    12.0,
                )],
                total_quantity: None,
                truncate_decimal: false,
                fixed_ratios: None,
                rank_interaction: None,
                price_factor: None,
                price_robustness: None,
                shovel_tons: None,
                min_ratios: Default::default(),
            };
            solve(&request).ok
        };

        assert!(solves(12.0), "正好压拒收线应可行");
        assert!(!solves(12.01), "超拒收线 0.01% (化验一个刻度) 必须判不可行");
    }

    /// 回归: 体检复核的容限必须与 LP 拒收行同量级.
    ///
    /// LP 的拒收行放行 `FEASIBILITY_TOLERANCE * (1 + magnitude)`, 于是最优解可能把
    /// 混合值顶到拒收线上、再高出浮点噪声那一丝. 体检的 `Fail → TolerancePass` 复核
    /// 若仍用绝对 `SOLUTION_TOLERANCE` (1e-8), 就会把 LP 认可的解判成 Fail,
    /// 再经 `finalize_quality_status` 变成 `NeedsReview` —— 正确配方被盖上"需要复核".
    ///
    /// 本例: 便宜脏煤(灰 13.25) + 干净贵煤(灰 8.0), 拒收线 12.75. 最优解顶在线上,
    /// 复算值 12.75000003279332539, 超线 3.28e-8 —— 旧的绝对 1e-8 兜不住.
    #[test]
    fn test_priced_indicator_check_tolerance_matches_lp_wall() {
        let request = BlendRequest {
            coals: vec![
                coal_from_tuple(
                    "脏便宜",
                    (1.0, 13.25, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1500.0, 0.0),
                ),
                coal_from_tuple(
                    "净贵",
                    (1.0, 8.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 2000.0, 0.0),
                ),
            ],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 5.0,
                }],
                12.75,
            )],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        let ash = result
            .indicator_check
            .iter()
            .find(|check| check.indicator == "A")
            .expect("应有灰分体检");
        assert!(
            ash.value > 12.75,
            "本用例须让复算值高出拒收线一丝才有意义, 实得 {}",
            ash.value
        );
        assert_ne!(
            ash.status,
            EvaluationStatus::Fail,
            "LP 已认可的解不应被体检判 Fail (复算值 {}, 拒收线 12.75)",
            ash.value
        );
        assert_ne!(
            result.quality_status,
            QualityStatus::NeedsReview,
            "正确配方不应被盖上需要复核"
        );
    }

    // ========================================================================
    // 买入侧: 按采购合同扣款与水分折算修正到厂价
    // ========================================================================

    fn purchase_ash_terms(guarantee: f64, rate: f64, reject: f64) -> PurchaseTerms {
        PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee,
                penalty: Penalty {
                    tiers: vec![PenaltyTier { width: None, rate }],
                    reject,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        }
    }

    /// 买入侧扣款是折扣: 让净成本**下降**, 与卖出侧方向相反.
    #[test]
    fn test_purchase_deduction_lowers_net_cost() {
        let mut coal = coal_from_tuple(
            "甲",
            (1.0, 10.5, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 100.0),
        );
        coal.purchase_terms = Some(purchase_ash_terms(10.0, 80.0, 12.0));

        let request = BlendRequest {
            coals: vec![coal],
            specs: Vec::new(),
            total_quantity: Some(10.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.cif_per_ton - 1100.0).abs() < 1e-6,
            "cif 应保持报价原值"
        );
        assert!(
            (cost.purchase_adjust_per_ton + 40.0).abs() < 1e-6,
            "买入修正应为 −40, 实得 {}",
            cost.purchase_adjust_per_ton
        );
        assert!(
            (cost.net_per_ton - 1060.0).abs() < 1e-6,
            "净成本应为 1060, 实得 {}",
            cost.net_per_ton
        );
        assert!(
            cost.total_net
                .is_some_and(|value| (value - 10600.0).abs() < 1e-4),
            "净成本总额应为 10600, 实得 {:?}",
            cost.total_net
        );
        assert!(
            cost.total_purchase_adjust
                .is_some_and(|value| (value + 400.0).abs() < 1e-4),
            "买入修正总额应为 −400, 实得 {:?}",
            cost.total_purchase_adjust
        );

        let order = result.orders.first().expect("应有订单");
        assert!(
            (order.cif_eff_per_ton - 1060.0).abs() < 1e-6,
            "订单单价应为修正后价, 实得 {}",
            order.cif_eff_per_ton
        );
    }

    /// 越过采购合同拒收线的煤被剔出煤池并留下 warning.
    #[test]
    fn test_coal_beyond_purchase_reject_is_dropped() {
        let mut dirty = coal_from_tuple(
            "脏煤",
            (1.0, 13.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 900.0, 0.0),
        );
        dirty.purchase_terms = Some(purchase_ash_terms(10.0, 80.0, 12.0));
        let clean = coal_from_tuple(
            "好煤",
            (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1200.0, 0.0),
        );

        let request = BlendRequest {
            coals: vec![dirty, clean],
            specs: Vec::new(),
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "剩余煤应仍可解");
        assert!(
            !result.recipe.contains_key("脏煤"),
            "越拒收线的煤不应入配方"
        );
        let warning = result
            .warnings
            .iter()
            .find(|warning| warning.contains("脏煤"))
            .unwrap_or_else(|| panic!("应有剔除警告, 实得 {:?}", result.warnings));
        // 与同处的"缺指标"分支对齐: 说清是哪条条款、实测多少、线在哪.
        assert!(
            warning.contains("灰") && warning.contains("13") && warning.contains("12"),
            "剔除告警应点名指标/实测值/拒收线, 实得 {warning}"
        );
    }

    /// 符号哨兵: 买入扣款把钱**少付**掉, 修正额必须严格为负、净成本严格低于到厂价.
    /// 符号写反时每个数字都仍然"看着合理", 只有这条断言会响.
    #[test]
    fn test_purchase_adjust_sign_is_strictly_a_discount() {
        let mut coal = coal_from_tuple(
            "甲",
            (1.0, 11.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 100.0),
        );
        coal.purchase_terms = Some(purchase_ash_terms(10.0, 80.0, 12.0));

        let request = BlendRequest {
            coals: vec![coal],
            specs: Vec::new(),
            total_quantity: Some(100.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert!(
            cost.purchase_adjust_per_ton < 0.0,
            "买入修正必须是折扣(负数), 实得 {}",
            cost.purchase_adjust_per_ton
        );
        assert!(
            cost.net_per_ton < cost.cif_per_ton,
            "被扣款的煤净成本必须低于到厂价, 实得 net {} vs cif {}",
            cost.net_per_ton,
            cost.cif_per_ton
        );
        assert!(
            cost.total_purchase_adjust.is_some_and(|value| value < 0.0),
            "买入修正总额同样必须为负, 实得 {:?}",
            cost.total_purchase_adjust
        );
        assert!(
            cost.total_net < cost.total_cif,
            "净成本总额必须低于到厂价总额, 实得 {:?} vs {:?}",
            cost.total_net,
            cost.total_cif
        );
    }

    /// 修正必须进 LP 目标, 不能只进报表: 报价更贵但扣款后更便宜的煤应被选中.
    /// 只改三视图、忘了改成本系数时, 上面几条仍会全绿, 只有这条会红.
    #[test]
    fn test_purchase_discount_changes_lp_choice() {
        let cheap_quote = coal_from_tuple(
            "报价便宜",
            (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
        );
        let mut discounted = coal_from_tuple(
            "扣款后便宜",
            (1.0, 11.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1050.0, 0.0),
        );
        // 灰分超保证 1.0, 80 元/吨·% ⇒ 扣 80 ⇒ 到厂价 1050 → 970, 反超 1000.
        discounted.purchase_terms = Some(purchase_ash_terms(10.0, 80.0, 12.0));

        let request = BlendRequest {
            coals: vec![cheap_quote, discounted],
            specs: Vec::new(),
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);
        assert!(
            result
                .recipe
                .get("扣款后便宜")
                .is_some_and(|ratio| (ratio - 1.0).abs() < 1e-4),
            "LP 应按修正后价选煤, 实得配方 {:?}",
            result.recipe
        );

        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.net_per_ton - 970.0).abs() < 1e-3,
            "净成本应为 970, 实得 {}",
            cost.net_per_ton
        );
    }

    /// 两侧同时生效: 买入折扣与卖出扣款方向相反, 净成本必须是三项代数和.
    /// 甲 A=10.5: 买入保证 10.0 扣 0.5×80 = 40 (降), 卖出合同界 10.0 扣 0.5×30 = 15 (升).
    #[test]
    fn test_both_sides_compose_in_opposite_directions() {
        let mut coal = coal_from_tuple(
            "甲",
            (1.0, 10.5, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
        );
        coal.purchase_terms = Some(purchase_ash_terms(10.0, 80.0, 12.0));

        let request = BlendRequest {
            coals: vec![coal],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier {
                    width: None,
                    rate: 30.0,
                }],
                13.0,
            )],
            total_quantity: Some(100.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "两侧同开应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert!(
            (cost.cif_per_ton - 1000.0).abs() < 1e-6,
            "cif 应保持报价原值, 实得 {}",
            cost.cif_per_ton
        );
        assert!(
            (cost.purchase_adjust_per_ton + 40.0).abs() < 1e-3,
            "买入修正应为 −40, 实得 {}",
            cost.purchase_adjust_per_ton
        );
        assert!(
            (cost.penalty_per_ton - 15.0).abs() < 1e-3,
            "卖出扣款应为 +15, 实得 {}",
            cost.penalty_per_ton
        );
        // 方向相反: 一负一正, 不是同号叠加.
        assert!(
            cost.purchase_adjust_per_ton < 0.0 && cost.penalty_per_ton > 0.0,
            "两侧应反向, 实得 买入 {} / 卖出 {}",
            cost.purchase_adjust_per_ton,
            cost.penalty_per_ton
        );
        assert!(
            (cost.net_per_ton
                - (cost.cif_per_ton + cost.purchase_adjust_per_ton + cost.penalty_per_ton))
                .abs()
                < 1e-6,
            "净成本必须是三项代数和, 实得 {}",
            cost.net_per_ton
        );
        assert!(
            (cost.net_per_ton - 975.0).abs() < 1e-3,
            "净成本应为 1000 − 40 + 15 = 975, 实得 {}",
            cost.net_per_ton
        );
    }

    /// 水分折算只打 fob 的折, 运费按实收湿重全额付 —— 湿煤是净亏运费, 不是中性.
    #[test]
    fn test_moisture_discounts_fob_but_not_freight_end_to_end() {
        let mut coal = coal_from_tuple(
            "湿煤",
            (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 10.0, 1000.0, 100.0),
        );
        coal.purchase_terms = Some(PurchaseTerms {
            clauses: Vec::new(),
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: None,
        });

        let request = BlendRequest {
            coals: vec![coal],
            specs: Vec::new(),
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        // 只折 fob: 1000×(0.90/0.92) + 100 = 1078.2609; 若连运费一起折则是 1100×(0.90/0.92) = 1076.0870.
        let fob_only = 1000.0 * (0.90 / 0.92) + 100.0;
        let if_freight_also_discounted = 1100.0 * (0.90 / 0.92);
        assert!(
            (cost.net_per_ton - fob_only).abs() < 1e-4,
            "净成本应为 {fob_only} (只折 fob), 实得 {}",
            cost.net_per_ton
        );
        assert!(
            (cost.net_per_ton - if_freight_also_discounted).abs() > 1.0,
            "净成本不应等于连运费一起折的 {if_freight_also_discounted}"
        );
        assert!(
            (cost.frt_per_ton - 100.0).abs() < 1e-9,
            "运费展示值必须是报价原值, 实得 {}",
            cost.frt_per_ton
        );
        assert!(
            (cost.purchase_adjust_per_ton - 1000.0 * (0.90 / 0.92 - 1.0)).abs() < 1e-4,
            "修正额应恰等于 fob 的折扣部分, 实得 {}",
            cost.purchase_adjust_per_ton
        );
    }

    /// 无采购条款时买入修正必须是**精确** 0, 不是 1e-13 量级的浮点渣.
    ///
    /// Σ(fob+frt)·x 与 Σfob·x + Σfrt·x 数学上相等、浮点上不等; 两个千元级大数相减
    /// 得到的"零"会带上残差, 再乘以总吨数放大. 修正额必须逐煤按 (修正价−报价) 算,
    /// 无条款时每项恰为 0.0·x = 0.0, 求和仍是精确 0.
    #[test]
    fn test_no_purchase_terms_yields_exactly_zero_adjust() {
        let request = BlendRequest {
            coals: vec![
                coal_from_tuple(
                    "甲",
                    (1.0, 12.3, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.39, 77.21),
                ),
                coal_from_tuple(
                    "乙",
                    (1.0, 6.1, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1400.87, 120.33),
                ),
            ],
            specs: vec![Spec::upper("A", 8.7)],
            total_quantity: Some(123457.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);
        assert_eq!(result.recipe.len(), 2, "本用例须真正混配才有意义");

        let cost = result.cost.expect("应有成本");
        assert_eq!(
            cost.purchase_adjust_per_ton, 0.0,
            "无采购条款时买入修正必须精确为 0, 实得 {:e}",
            cost.purchase_adjust_per_ton
        );
        assert_eq!(cost.total_purchase_adjust, Some(0.0));
        assert_eq!(
            cost.net_per_ton, cost.cif_per_ton,
            "净成本应与到厂价逐位相等"
        );
    }

    /// 买入侧修正必须穿过 solve_json 的字符串契约 —— 这是服务端与命令行实际走的唯一入口.
    ///
    /// 上面几条测的是 Rust 结构体路径; 存量测试只覆盖了 purchase_terms **缺席**的老 JSON,
    /// 它**在场**时的 serde 字段名从未被验证过. 任一字段名对不上, 结构体路径全绿而
    /// 线上静默按报价原值求解.
    /// 甲: 水分 10% 超双倍阈值 9% ⇒ M_eff = 2×10−9 = 11%; 灰分超保证 0.5 ⇒ 扣 40.
    /// 到厂价 = 1000×(0.89/0.92) + 100 − 40 = 1027.3913.
    #[test]
    fn test_solve_json_applies_purchase_terms() {
        let payload = r#"{
            "coals": [
                {
                    "name":"甲",
                    "props":{"S":1.0,"A":10.5,"V":24.0,"G":88.0,"Y":16.0,"petro":0.10,"CSR":65.0,"M":10.0},
                    "fob":1000.0,
                    "frt":100.0,
                    "purchase_terms":{
                        "clauses":[{
                            "indicator":"A",
                            "direction":"Upper",
                            "guarantee":10.0,
                            "penalty":{"tiers":[{"rate":80.0}],"reject":12.0}
                        }],
                        "contract_moisture":8.0,
                        "moisture_excess_double_threshold":9.0
                    }
                }
            ],
            "specs": [],
            "total_quantity": 100.0,
            "truncate_decimal": false
        }"#;
        let output = solve_json(payload);
        let expected_cif_eff = 1000.0 * (0.89 / 0.92) + 100.0 - 40.0;

        // 断言必须落在**实际 JSON 键**上: 反序列化回 BlendResult 会让 #[serde(rename)]
        // 原样往返, 测试照样绿而前端拿不到字段. 这个 core 的契约是字符串进字符串出,
        // 契约就是这些键名本身.
        let json: serde_json::Value = serde_json::from_str(&output).expect("输出应为合法 JSON");
        let key = |value: &serde_json::Value, path: &[&str]| -> f64 {
            let mut node = value;
            for step in path {
                node = node
                    .get(step)
                    .unwrap_or_else(|| panic!("输出 JSON 缺少键 {}", path.join(".")));
            }
            node.as_f64()
                .unwrap_or_else(|| panic!("{} 应为数值, 实得 {node}", path.join(".")))
        };

        assert!(json["ok"].as_bool().unwrap_or(false), "应可求解: {output}");
        assert!(
            (key(&json, &["cost", "cif_per_ton"]) - 1100.0).abs() < 1e-6,
            "cost.cif_per_ton 应保持报价原值"
        );
        assert!(
            (key(&json, &["cost", "net_per_ton"]) - expected_cif_eff).abs() < 1e-6,
            "cost.net_per_ton 应为 {expected_cif_eff} —— 为 1100 说明 purchase_terms 没被解析"
        );
        assert!(
            (key(&json, &["cost", "purchase_adjust_per_ton"]) - (expected_cif_eff - 1100.0)).abs()
                < 1e-6,
            "cost.purchase_adjust_per_ton 应为 {}",
            expected_cif_eff - 1100.0
        );
        let order = &json["orders"][0];
        assert!(
            (key(order, &["cif_eff_per_ton"]) - expected_cif_eff).abs() < 1e-6,
            "orders[0].cif_eff_per_ton 应为修正后价"
        );
        assert!(
            (key(order, &["cif_eff_amount"]) - expected_cif_eff * 100.0).abs() < 1e-4,
            "orders[0].cif_eff_amount 应为修正后价 × 100 吨"
        );

        // 结构体路径同样要通, 且两条路径必须给出同一个数.
        let result: BlendResult = serde_json::from_str(&output).expect("结果应可反序列化");
        let cost = result.cost.expect("应有成本");
        assert!((cost.net_per_ton - expected_cif_eff).abs() < 1e-6);
    }

    /// 订单金额口径必须自洽: 每行 cif_eff_amount = cif_eff_per_ton × tons,
    /// 且 Σ cif_eff_amount + 卖出扣款 = total_net.
    /// cif_amount 是报价口径展示值, 两者不可混用 —— 前端取错会静默吞掉买入折扣.
    #[test]
    fn test_order_effective_amount_reconciles_with_total_net() {
        let mut coal = coal_from_tuple(
            "甲",
            (1.0, 10.5, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 100.0),
        );
        coal.purchase_terms = Some(purchase_ash_terms(10.0, 80.0, 12.0));
        let clean = coal_from_tuple(
            "乙",
            (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1080.0, 0.0),
        );

        let request = BlendRequest {
            coals: vec![coal, clean],
            specs: vec![Spec::upper("A", 10.0)],
            total_quantity: Some(200.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        for order in &result.orders {
            let tons = order.tons.expect("应有吨数");
            let amount = order.cif_eff_amount.expect("应有修正后金额");
            assert!(
                (amount - order.cif_eff_per_ton * tons).abs() < 1e-6,
                "{} 行内不自洽: {} ≠ {} × {}",
                order.coal,
                amount,
                order.cif_eff_per_ton,
                tons
            );
        }

        let cost = result.cost.expect("应有成本");
        let summed: f64 = result
            .orders
            .iter()
            .map(|order| order.cif_eff_amount.unwrap_or(0.0))
            .sum();
        let total_net = cost.total_net.expect("应有净成本总额");
        let total_penalty = cost.total_penalty.expect("应有卖出扣款总额");
        assert!(
            (summed + total_penalty - total_net).abs() < 1e-3,
            "Σ cif_eff_amount({summed}) + 卖出扣款({total_penalty}) 应等于 total_net({total_net})"
        );
        // 报价口径的合计与净额不同 —— 正是前端取错字段会丢掉的那部分.
        let quoted: f64 = result
            .orders
            .iter()
            .map(|order| order.cif_amount.unwrap_or(0.0))
            .sum();
        assert!(
            quoted > summed,
            "报价口径合计({quoted})应高于修正后合计({summed}), 否则本用例没覆盖到折扣"
        );
    }

    /// 同一份档位表, 卖出侧走 LP 档位列、买入侧走 tiered_amount 算术, 两条实现必须
    /// 算出同样的钱. 不统一代码是刻意的(凸性这个共同前提已集中在 validate_penalty),
    /// 但一致性得有东西钉着.
    #[test]
    fn test_buy_and_sell_sides_price_the_same_schedule_alike() {
        let tiers = || {
            vec![
                PenaltyTier {
                    width: Some(1.0),
                    rate: 10.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 30.0,
                },
            ]
        };
        // 灰分 11.5, 两侧界都是 10.0 ⇒ 偏离 1.5 ⇒ 一档 1.0×10 + 二档 0.5×30 = 25.
        let mut coal = coal_from_tuple(
            "甲",
            (1.0, 11.5, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
        );
        coal.purchase_terms = Some(PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: tiers(),
                    reject: 13.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        });

        let request = BlendRequest {
            coals: vec![coal],
            specs: vec![priced_upper_spec("A", 10.0, tiers(), 13.0)],
            total_quantity: Some(100.0),
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        let bought = -cost.purchase_adjust_per_ton;
        let sold = cost.penalty_per_ton;
        assert!(
            (bought - 25.0).abs() < 1e-3,
            "买入侧算术扣款应为 25, 实得 {bought}"
        );
        assert!(
            (sold - 25.0).abs() < 1e-3,
            "卖出侧 LP 扣款应为 25, 实得 {sold}"
        );
        assert!(
            (bought - sold).abs() < 1e-3,
            "两条实现对同一份档位表必须逐分一致, 实得 买入 {bought} / 卖出 {sold}"
        );
        // 大小相等方向相反 ⇒ 净成本回到报价.
        assert!(
            (cost.net_per_ton - cost.cif_per_ton).abs() < 1e-3,
            "两侧等额相消后净成本应回到 cif, 实得 {} vs {}",
            cost.net_per_ton,
            cost.cif_per_ton
        );
    }

    /// 采购条款缺化验值: 煤留着(全局模板下缺项是常态), 但必须告警说明没计进扣款.
    #[test]
    fn test_clause_missing_assay_warns_but_keeps_coal() {
        let mut coal = coal_from_tuple(
            "甲",
            (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0),
        );
        coal.props.remove("S");
        coal.purchase_terms = Some(PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "S".into(),
                direction: Direction::Upper,
                guarantee: 1.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 50.0,
                    }],
                    reject: 2.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        });

        let request = BlendRequest {
            coals: vec![coal],
            specs: Vec::new(),
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        };
        let result = solve(&request);
        assert!(result.ok, "缺化验值不应让煤不可用: {:?}", result.reason);
        assert!(result.recipe.contains_key("甲"), "煤应留在池里");
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("甲") && warning.contains("硫")),
            "应有缺化验值告警并点名指标, 实得 {:?}",
            result.warnings
        );
        let cost = result.cost.expect("应有成本");
        assert_eq!(
            cost.purchase_adjust_per_ton, 0.0,
            "算不出的条款不该凭空产生修正"
        );
    }
}
