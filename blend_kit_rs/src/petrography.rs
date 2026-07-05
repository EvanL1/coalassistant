//! 配煤岩相学: 混煤反射率分布的精确计算.
//!
//! 调研结论 (docs/superpowers/specs/2026-07-04-nonlinear-blend-research.md §3):
//!   - 混煤直方图 = 单煤直方图按 y_j 加权合成, y_j = V_j·x_j / Σ(V_k·x_k)
//!     (V_j = 镜质组含量; 权重不是质量配比本身, 仅当各煤 V 相同时才退化为 x_j)
//!   - 混合 μ/σ 按直方图离散二阶矩计算 (全方差定律), σ 对配比是非线性的,
//!     线性加权 σ_j 会系统性低估混煤 σ
//!   - 直方图凹口落在主焦煤区间 1.2~1.5 损害焦质 (本钢 40kg 焦炉实测)
use serde::{Deserialize, Serialize};

/// 单煤煤岩数据 (来自 MT/T 507 煤岩化验单).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Petrography {
    /// 镜质体反射率直方图: [bin 中值 R(%), 频率] 列表.
    /// 频率允许未归一化 (计数或百分比均可), 计算前统一归一化.
    pub hist: Vec<[f64; 2]>,
    /// 镜质组体积含量 (%), 用于混合权重修正.
    pub vitrinite_pct: f64,
}

impl Petrography {
    /// 数据可用于混合计算: 镜质组含量为正且直方图有正频率.
    /// 无效数据必须视同"缺数据"处理, 否则会静默算出只覆盖部分参配煤的"精确" σ.
    pub fn is_valid(&self) -> bool {
        self.vitrinite_pct > 0.0 && self.hist.iter().any(|b| b[1] > 0.0)
    }
}

/// 凹口检测结果.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notch {
    /// 谷底所在反射率.
    pub r: f64,
    /// 谷深比 = 谷频率 / min(左峰, 右峰). 0 = 完全断档.
    pub depth_ratio: f64,
}

/// 主焦煤敏感区间: 凹口落在此区间直接损害焦质 (本钢《金属世界》2019 实测).
pub const NOTCH_WINDOW: (f64, f64) = (1.2, 1.5);
/// 凹口判定阈值: 谷频率低于两侧峰较小者的一半判为凹口.
pub const NOTCH_RATIO: f64 = 0.5;

/// 直方图的均值与标准差 (离散二阶矩). 空直方图或总频率为 0 → None.
pub fn hist_mean_std(hist: &[[f64; 2]]) -> Option<(f64, f64)> {
    let total: f64 = hist.iter().map(|b| b[1]).sum();
    if total <= 0.0 {
        return None;
    }
    let mean: f64 = hist.iter().map(|b| b[0] * b[1]).sum::<f64>() / total;
    let var: f64 = hist
        .iter()
        .map(|b| (b[0] - mean).powi(2) * b[1])
        .sum::<f64>()
        / total;
    // 浮点误差可能产生 -1e-18 级微负
    Some((mean, var.max(0.0).sqrt()))
}

/// 混煤直方图合成: parts = [(单煤煤岩, 质量配比 x_j)].
/// 权重 y_j = vitrinite_pct_j·x_j / Σ(vitrinite_pct_k·x_k); 各煤直方图先归一化再加权合并.
/// Σ(V·x) = 0 或 parts 为空 → None.
pub fn mix_histogram(parts: &[(&Petrography, f64)]) -> Option<Vec<[f64; 2]>> {
    let vx_total: f64 = parts.iter().map(|(p, x)| p.vitrinite_pct * x).sum();
    if vx_total <= 0.0 {
        return None;
    }
    // bin 中值按 1e-4 精度合并 (反射率化验单通常两位小数)
    let mut merged: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (p, x) in parts {
        let y = p.vitrinite_pct * x / vx_total;
        let coal_total: f64 = p.hist.iter().map(|b| b[1]).sum();
        if coal_total <= 0.0 {
            continue;
        }
        for b in &p.hist {
            // 化验单常枚举全量程含 0% 行; 零频率 bin 不携带质量,
            // 混入会让凹口检测把 0 bin 当谷底产生假"断档"
            if b[1] <= 0.0 {
                continue;
            }
            let key = (b[0] * 1e4).round() as i64;
            *merged.entry(key).or_insert(0.0) += y * b[1] / coal_total;
        }
    }
    if merged.is_empty() {
        return None;
    }
    Some(
        merged
            .into_iter()
            .map(|(k, w)| [k as f64 / 1e4, w])
            .collect(),
    )
}

/// 在 [lo, hi] 窗口内检测凹口.
/// 规则: 窗口两侧 (r < lo 与 r > hi) 都存在正频率峰时, 取窗口内最低频率为谷
/// (窗口内无 bin 视为频率 0 的完全断档), 谷 < NOTCH_RATIO × min(左峰, 右峰) 判凹口.
pub fn detect_notch(hist: &[[f64; 2]], lo: f64, hi: f64) -> Option<Notch> {
    let left_peak = hist
        .iter()
        .filter(|b| b[0] < lo)
        .map(|b| b[1])
        .fold(0.0_f64, f64::max);
    let right_peak = hist
        .iter()
        .filter(|b| b[0] > hi)
        .map(|b| b[1])
        .fold(0.0_f64, f64::max);
    if left_peak <= 0.0 || right_peak <= 0.0 {
        return None; // 单侧无质量 → 不构成"凹口" (分布本身就不覆盖该区间)
    }
    // 窗口内最低 bin 为谷; 无 bin = 完全断档 (谷频率 0, 谷位取窗口中点)
    let valley = hist
        .iter()
        .filter(|b| b[0] >= lo && b[0] <= hi)
        .min_by(|a, b| a[1].partial_cmp(&b[1]).unwrap())
        .map(|b| (b[0], b[1]))
        .unwrap_or(((lo + hi) / 2.0, 0.0));
    let depth_ratio = valley.1 / left_peak.min(right_peak);
    if depth_ratio < NOTCH_RATIO {
        Some(Notch {
            r: valley.0,
            depth_ratio,
        })
    } else {
        None
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn petro(hist: Vec<[f64; 2]>, vitrinite_pct: f64) -> Petrography {
        Petrography {
            hist,
            vitrinite_pct,
        }
    }

    #[test]
    fn test_hist_mean_std_single_bin() {
        let (mean, std) = hist_mean_std(&[[1.2, 100.0]]).unwrap();
        assert!((mean - 1.2).abs() < 1e-9);
        assert!(std.abs() < 1e-9);
    }

    #[test]
    fn test_hist_mean_std_two_bins() {
        // 等频双 bin: μ = 1.5, σ = 0.5
        let (mean, std) = hist_mean_std(&[[1.0, 1.0], [2.0, 1.0]]).unwrap();
        assert!((mean - 1.5).abs() < 1e-9);
        assert!((std - 0.5).abs() < 1e-9);
        // 未归一化频率 (计数 2/2) 结果不变
        let (mean2, std2) = hist_mean_std(&[[1.0, 2.0], [2.0, 2.0]]).unwrap();
        assert!((mean2 - 1.5).abs() < 1e-9);
        assert!((std2 - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_hist_mean_std_empty_or_zero() {
        assert!(hist_mean_std(&[]).is_none());
        assert!(hist_mean_std(&[[1.0, 0.0]]).is_none());
    }

    #[test]
    fn test_mix_histogram_equal_vitrinite() {
        // 镜质组含量相同 → 权重退化为质量配比
        let a = petro(vec![[1.0, 1.0]], 80.0);
        let b = petro(vec![[2.0, 1.0]], 80.0);
        let mixed = mix_histogram(&[(&a, 0.5), (&b, 0.5)]).unwrap();
        let (mean, std) = hist_mean_std(&mixed).unwrap();
        assert!((mean - 1.5).abs() < 1e-9);
        assert!((std - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_mix_histogram_vitrinite_correction() {
        // V=80 vs V=40, 质量各半 → y_A = 2/3, y_B = 1/3 (调研 §3.1 权重修正)
        let a = petro(vec![[0.9, 1.0]], 80.0);
        let b = petro(vec![[1.5, 1.0]], 40.0);
        let mixed = mix_histogram(&[(&a, 0.5), (&b, 0.5)]).unwrap();
        let (mean, _) = hist_mean_std(&mixed).unwrap();
        let expected = 0.9 * (2.0 / 3.0) + 1.5 * (1.0 / 3.0);
        assert!(
            (mean - expected).abs() < 1e-9,
            "mean={} expected={}",
            mean,
            expected
        );
    }

    /// 文档性测试: 单煤 σ 全为 0 时混煤 σ 仍显著为正 —— 线性加权 σ 必然低估.
    #[test]
    fn test_mix_sigma_exceeds_weighted_sigma() {
        let a = petro(vec![[0.9, 1.0]], 80.0); // σ_A = 0
        let b = petro(vec![[1.6, 1.0]], 80.0); // σ_B = 0
        let mixed = mix_histogram(&[(&a, 0.5), (&b, 0.5)]).unwrap();
        let (_, std) = hist_mean_std(&mixed).unwrap();
        assert!(
            (std - 0.35).abs() < 1e-9,
            "混煤 σ 应 = 0.35 (μ 离散贡献), 实际 {}",
            std
        );
    }

    #[test]
    fn test_mix_histogram_merges_same_bins() {
        // 两煤有重叠 bin → 频率合并, 不产生重复 bin
        let a = petro(vec![[1.0, 1.0], [1.1, 1.0]], 80.0);
        let b = petro(vec![[1.1, 1.0], [1.2, 1.0]], 80.0);
        let mixed = mix_histogram(&[(&a, 0.5), (&b, 0.5)]).unwrap();
        assert_eq!(mixed.len(), 3, "重叠 bin 应合并: {:?}", mixed);
    }

    #[test]
    fn test_mix_histogram_zero_vitrinite() {
        let a = petro(vec![[1.0, 1.0]], 0.0);
        assert!(mix_histogram(&[(&a, 1.0)]).is_none());
        assert!(mix_histogram(&[]).is_none());
    }

    #[test]
    fn test_detect_notch_full_gap() {
        // 双峰在窗口两侧, 窗口内无 bin → 完全断档, depth_ratio = 0
        let hist = [[0.9, 5.0], [1.0, 3.0], [1.6, 4.0], [1.7, 2.0]];
        let notch = detect_notch(&hist, 1.2, 1.5).unwrap();
        assert!(notch.depth_ratio.abs() < 1e-9, "断档 depth_ratio 应为 0");
        assert!(notch.r >= 1.2 && notch.r <= 1.5);
    }

    #[test]
    fn test_detect_notch_valley_in_window() {
        // 窗口内有低谷 (1.35 频率 1, 两侧峰 5/4) → 谷深比 1/4 < 0.5 判凹口
        let hist = [[1.0, 5.0], [1.35, 1.0], [1.7, 4.0]];
        let notch = detect_notch(&hist, 1.2, 1.5).unwrap();
        assert!((notch.r - 1.35).abs() < 1e-9);
        assert!((notch.depth_ratio - 0.25).abs() < 1e-9);
    }

    #[test]
    fn test_detect_notch_shallow_valley_is_not_notch() {
        // 谷 3 / min(峰 5, 峰 4) = 0.75 ≥ 0.5 → 不算凹口
        let hist = [[1.0, 5.0], [1.35, 3.0], [1.7, 4.0]];
        assert!(detect_notch(&hist, 1.2, 1.5).is_none());
    }

    /// 化验单常按 0.05 步长枚举全量程 (含大量 0% 行). 零频率 bin 不得进入合成直方图,
    /// 否则凹口检测的谷底会选中 0 bin, 对物理上无凹口的分布误报"完全断档".
    #[test]
    fn test_mix_drops_zero_bins_no_false_notch() {
        // A 在窗口 1.3 处有充足质量 (混合后谷深比 ≈ 0.63 ≥ 0.5, 物理上无凹口)
        let a = petro(vec![[1.0, 4.0], [1.3, 6.0], [1.7, 4.0]], 80.0);
        // B 的化验单带显式 0% 行 (1.35 处) —— 若不剔除, 谷底会选中它误报完全断档
        let b = petro(vec![[1.0, 3.0], [1.35, 0.0], [1.7, 2.0]], 80.0);
        let mixed = mix_histogram(&[(&a, 0.5), (&b, 0.5)]).unwrap();
        assert!(
            mixed.iter().all(|bin| bin[1] > 0.0),
            "合成直方图不应含零权重 bin: {:?}",
            mixed
        );
        assert!(
            detect_notch(&mixed, 1.2, 1.5).is_none(),
            "零频率行不应制造假凹口"
        );
    }

    /// 数据有效性判定: 直方图空/全零频率/镜质组含量非正 → 无效.
    #[test]
    fn test_petrography_is_valid() {
        assert!(petro(vec![[1.2, 1.0]], 80.0).is_valid());
        assert!(!petro(vec![], 80.0).is_valid(), "空直方图无效");
        assert!(!petro(vec![[1.2, 0.0]], 80.0).is_valid(), "全零频率无效");
        assert!(
            !petro(vec![[1.2, 1.0]], 0.0).is_valid(),
            "镜质组含量 0 无效"
        );
    }

    #[test]
    fn test_detect_notch_requires_both_flanks() {
        // 全部质量在窗口左侧 → 无右峰 → 不判凹口
        let hist = [[0.9, 5.0], [1.0, 3.0], [1.1, 1.0]];
        assert!(detect_notch(&hist, 1.2, 1.5).is_none());
    }
}
