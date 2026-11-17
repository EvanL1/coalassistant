//! 焦炭 CSR 的两样东西, 都不进求解:
//!
//! - **实测 CSR 的回归拟合** ([`CsrPredictor::fit`]): 用历史配煤的
//!   [六项混合指标, 实测 CSR] 做最小二乘,
//!   `CSR = β₀ + β_S·S + β_A·A + β_V·V + β_G·G + β_Y·Y + β_M·M`.
//!   只给后台的「CSR 模型」状态页看样本进度与拟合优度; 拟合出来的系数不参与配方.
//! - **捣固炼焦的 CSR 估算** ([`csr_stamp_charging`]): 由配合煤的挥发 / G / Y 按经验式算,
//!   每次求解都附在结果里 (`BlendResult.csr_stamp_estimate`), 只展示.

use serde::{Deserialize, Serialize};

/// 单次历史配煤观测记录. 六项特征是保存方案时按配比加权的混合指标,
/// 不能逐字段混入事后实测值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsrObservation {
    pub s: f64,
    pub a: f64,
    pub v: f64,
    pub g: f64,
    pub y: f64,
    pub m: f64,
    pub csr_measured: f64,
}

/// 拟合出来的 CSR 线性式 (系数与拟合优度).
#[derive(Debug, Clone)]
pub struct CsrPredictor {
    pub intercept: f64,
    pub beta_s: f64,
    pub beta_a: f64,
    pub beta_v: f64,
    pub beta_g: f64,
    pub beta_y: f64,
    pub beta_m: f64,
    pub r_squared: f64,
    pub n_samples: usize,
}

impl CsrPredictor {
    /// 用最小二乘法拟合观测数据.
    ///
    /// 最少需要 7 个观测 (6 自变量 + 1 截距列). 矩阵奇异时返回 Err.
    pub fn fit(observations: &[CsrObservation]) -> Result<Self, String> {
        let n = observations.len();
        if n < 7 {
            return Err(format!("样本数不足: 需要至少 7 个观测, 实际 {}", n));
        }
        validate_csr_observations(observations)?;

        // 构造设计矩阵 X (n×7) 和目标向量 y (n).
        // 列顺序: [1, S, A, V, G, Y, M]
        const P: usize = 7;
        let mut xt_x = [[0f64; P]; P]; // XᵀX, 7×7
        let mut xt_y = [0f64; P]; // Xᵀy, 7

        for obs in observations {
            let row = [1.0, obs.s, obs.a, obs.v, obs.g, obs.y, obs.m];
            for i in 0..P {
                xt_y[i] += row[i] * obs.csr_measured;
                for j in 0..P {
                    xt_x[i][j] += row[i] * row[j];
                }
            }
        }

        // 用高斯-若尔当消元求解 (XᵀX) β = Xᵀy.
        // 增广矩阵 [A | b], 7×8.
        let mut aug = [[0f64; P + 1]; P];
        for i in 0..P {
            for j in 0..P {
                aug[i][j] = xt_x[i][j];
            }
            aug[i][P] = xt_y[i];
        }

        gauss_jordan(&mut aug)?;

        let beta = [
            aug[0][P], aug[1][P], aug[2][P], aug[3][P], aug[4][P], aug[5][P], aug[6][P],
        ];

        // 计算 R².
        let y_mean: f64 = observations.iter().map(|o| o.csr_measured).sum::<f64>() / n as f64;
        let ss_tot: f64 = observations
            .iter()
            .map(|o| (o.csr_measured - y_mean).powi(2))
            .sum();
        let ss_res: f64 = observations
            .iter()
            .map(|o| {
                let pred = beta[0]
                    + beta[1] * o.s
                    + beta[2] * o.a
                    + beta[3] * o.v
                    + beta[4] * o.g
                    + beta[5] * o.y
                    + beta[6] * o.m;
                (o.csr_measured - pred).powi(2)
            })
            .sum();

        let r_squared = if ss_tot < 1e-12 {
            1.0 // 所有 y 相同且残差为 0
        } else {
            1.0 - ss_res / ss_tot
        };
        // 输入都有限也可能在平方求和时溢出 (观测值极大), 结果不可用.
        if !r_squared.is_finite() || beta.iter().any(|value| !value.is_finite()) {
            return Err("CSR 观测数值过大, 拟合溢出".into());
        }

        Ok(CsrPredictor {
            intercept: beta[0],
            beta_s: beta[1],
            beta_a: beta[2],
            beta_v: beta[3],
            beta_g: beta[4],
            beta_y: beta[5],
            beta_m: beta[6],
            r_squared,
            n_samples: n,
        })
    }
}

fn validate_csr_observations(observations: &[CsrObservation]) -> Result<(), String> {
    if observations.iter().any(|observation| {
        csr_features(observation)
            .into_iter()
            .chain(std::iter::once(observation.csr_measured))
            .any(|value| !value.is_finite())
    }) {
        return Err("CSR 观测包含非法数值".into());
    }
    Ok(())
}

fn csr_features(observation: &CsrObservation) -> [f64; 6] {
    [
        observation.s,
        observation.a,
        observation.v,
        observation.g,
        observation.y,
        observation.m,
    ]
}

/// 对 7×8 增广矩阵做高斯-若尔当消元 (全主元), 求解 7 元线性方程组.
/// 就地修改, 解写回最后一列. 矩阵奇异时返回 Err.
fn gauss_jordan(aug: &mut [[f64; 8]; 7]) -> Result<(), String> {
    const N: usize = 7;
    for col in 0..N {
        // 选列最大主元 (部分主元). total_cmp: 溢出出 NaN 时也不会 panic, 由下面的有限性检查拦下.
        let pivot_row = (col..N)
            .max_by(|&a, &b| aug[a][col].abs().total_cmp(&aug[b][col].abs()))
            .unwrap_or(col);
        if !aug[pivot_row][col].is_finite() {
            return Err("数值溢出: 观测值过大".into());
        }
        if aug[pivot_row][col].abs() < 1e-12 {
            return Err("矩阵奇异: 自变量之间存在完全共线性".into());
        }
        aug.swap(col, pivot_row);

        let pivot = aug[col][col];
        for v in aug[col][col..=N].iter_mut() {
            *v /= pivot;
        }

        let pivot_row_vals = aug[col]; // [f64; 8] 是 Copy, 复制后避免行间借用冲突
        for (row, row_vals) in aug.iter_mut().enumerate() {
            if row == col {
                continue;
            }
            let factor = row_vals[col];
            for (v, &p) in row_vals[col..=N]
                .iter_mut()
                .zip(pivot_row_vals[col..=N].iter())
            {
                *v -= factor * p;
            }
        }
    }
    if aug.iter().any(|row| !row[N].is_finite()) {
        return Err("数值溢出: 观测值过大".into());
    }
    Ok(())
}

/// 煤灰成分 (占灰的质量百分数), 算矿物质催化指数 [`mineral_catalysis_index`] 用.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AshComposition {
    pub fe2o3: f64,
    pub k2o: f64,
    pub na2o: f64,
    pub cao: f64,
    pub mgo: f64,
    pub mno: f64,
    pub sio2: f64,
    pub al2o3: f64,
    pub tio2: f64,
}

/// 矿物质催化指数 MCI, **以百分数计**:
///
/// ```text
/// MCI = 100 × Ad × (Fe₂O₃ + 1.85K₂O + 2.2Na₂O + 1.6CaO + 0.83MgO + 0.9MnO)
///             / ((100 − Vd) × (SiO₂ + 0.41Al₂O₃ + 2.5TiO₂)),   Vd = Vdaf × (100 − Ad)/100
/// ```
///
/// 必须乘 100: MCI 的常见取值在 1~8, 基准 δ=1.0 也在这个量级; 不乘 100 的话
/// 典型灰成分只得 0.02 上下, 与之对不上。
/// 分子是促进焦炭溶损的碱性氧化物, 分母是惰性的酸性氧化物; Ad、Vdaf 以 % 计。
pub fn mineral_catalysis_index(ad: f64, vdaf: f64, ash: &AshComposition) -> f64 {
    let vd = vdaf * (100.0 - ad) / 100.0;
    let basic = ash.fe2o3
        + 1.85 * ash.k2o
        + 2.2 * ash.na2o
        + 1.6 * ash.cao
        + 0.83 * ash.mgo
        + 0.9 * ash.mno;
    let acidic = ash.sio2 + 0.41 * ash.al2o3 + 2.5 * ash.tio2;
    100.0 * ad * basic / ((100.0 - vd) * acidic)
}

/// 捣固炼焦的焦炭 CSR 估算 —— 与按单煤 CSR 线性加权并列展示的第二个估计, 不进 LP.
///
/// 以山西柳林煤为基准标定:
///
/// ```text
/// M40 = −1.14·Vdaf + 0.036·G + 1.13·Y + 87.5 + 50·ρ − 50
/// CSR = −0.032·M40² + 6.336·M40 − 241.18 − (MCI − 1.0) × 5.7
/// ```
///
/// - ρ 取 1.0 (t/m³): 产品默认捣固炼焦。
/// - `mci` 为 None 时不做 MCI 修正, 等于假设灰成分与柳林基准煤相当 (MCI = δ = 1.0)。
///   MCI 常见取值在 1~8、基准 1.0 在最低端, 大多数煤高于它 —— 所以不修正
///   对灰中碱金属多的煤会**高估** CSR (MCI 3 就差 11.4 个点)。目前没有灰成分数据,
///   调用方一律传 None; 有了灰成分再用 [`mineral_catalysis_index`] 算出来传入。
/// - 输入是配合煤的挥发/G/Y, 不看单煤 CSR, 分不出挥发/G/Y 相近而单煤 CSR 差很多的煤。
/// - 二次式在 M40 = 99 处取极大值约 72.5, 超过后反而下降, 不外推到那一侧去读。
///
/// 唯一的实测对照: 配方 2:4:3:2, 配合煤实测 Vdaf 22.14 / G 87 / Y 20, 焦炭实测
/// CSR 70; 本式 (不修正 MCI) 68.6, 线性加权 62.8。一个样本, 只是合理性检查。
pub fn csr_stamp_charging(vdaf: f64, g: f64, y: f64, mci: Option<f64>) -> f64 {
    const BULK_DENSITY: f64 = 1.0;
    const MCI_BASELINE: f64 = 1.0;
    const MCI_SLOPE: f64 = 5.7;
    let m40 = -1.14 * vdaf + 0.036 * g + 1.13 * y + 87.5 + 50.0 * BULK_DENSITY - 50.0;
    let correction = mci.map_or(0.0, |mci| (mci - MCI_BASELINE) * MCI_SLOPE);
    -0.032 * m40 * m40 + 6.336 * m40 - 241.18 - correction
}

/// [`csr_stamp_charging`] 加上适用范围检查: 外推出 0~100 以外的 CSR 是公式失效, 不是
/// 焦炭质量, 不给数, 返回一条说明. 高挥发低 G/Y 的气煤配方会落到这一侧 (M40 低于约 51.4
/// 时二次式为负).
pub fn csr_stamp_in_domain(vdaf: f64, g: f64, y: f64, mci: Option<f64>) -> Result<f64, String> {
    let estimate = csr_stamp_charging(vdaf, g, y, mci);
    if estimate.is_finite() && (0.0..=100.0).contains(&estimate) {
        return Ok(estimate);
    }
    Err(format!(
        "捣固 CSR 估算超出公式适用范围 (配合煤挥发 {vdaf:.2} / G {g:.1} / Y {y:.1} 外推得 \
         {estimate:.1}), 本次不给估算值"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成满足完全线性关系的观测: CSR = 30 + 1·S + 0.5·A + 0.8·V + 0.3·G + 0.6·Y + 0.4·M
    ///
    /// 6 个特征必须线性独立, 否则 X^T·X 奇异. 用确定性"伪随机"序列:
    /// 取一组无理数的小数部分作偏移, 保证彼此不相关.
    fn make_obs(n: usize) -> Vec<CsrObservation> {
        (0..n)
            .map(|i| {
                let t = i as f64;
                // 用不同的非线性映射拉开特征间相关性 (sin/cos/不同周期)
                let s = 1.5 + (t * 0.7).sin().abs() * 1.2;
                let a = 6.0 + (t * 1.3).cos().abs() * 2.5;
                let v = 18.0 + ((t * 0.5 + 1.0).sin().abs()) * 8.0;
                let g = 75.0 + (t * 0.9).cos().abs() * 18.0;
                let y = 10.0 + (t * 1.7).sin().abs() * 9.0;
                let m = 8.0 + ((t * 0.3 + 0.5).cos().abs()) * 3.0;
                let csr = 30.0 + 1.0 * s + 0.5 * a + 0.8 * v + 0.3 * g + 0.6 * y + 0.4 * m;
                CsrObservation {
                    s,
                    a,
                    v,
                    g,
                    y,
                    m,
                    csr_measured: csr,
                }
            })
            .collect()
    }

    #[test]
    fn test_fit_perfect_linear() {
        let obs = make_obs(10);
        let predictor = CsrPredictor::fit(&obs).expect("拟合应成功");

        assert!(
            predictor.r_squared > 0.999,
            "r_squared = {} (期望 > 0.999)",
            predictor.r_squared
        );
        assert!(
            (predictor.intercept - 30.0).abs() < 0.01,
            "intercept = {} (期望 ≈ 30.0)",
            predictor.intercept
        );
        assert!(
            (predictor.beta_s - 1.0).abs() < 0.01,
            "beta_s = {} (期望 ≈ 1.0)",
            predictor.beta_s
        );
        assert!(
            (predictor.beta_a - 0.5).abs() < 0.01,
            "beta_a = {} (期望 ≈ 0.5)",
            predictor.beta_a
        );
        assert!(
            (predictor.beta_v - 0.8).abs() < 0.01,
            "beta_v = {} (期望 ≈ 0.8)",
            predictor.beta_v
        );
        assert!(
            (predictor.beta_g - 0.3).abs() < 0.01,
            "beta_g = {} (期望 ≈ 0.3)",
            predictor.beta_g
        );
        assert!(
            (predictor.beta_y - 0.6).abs() < 0.01,
            "beta_y = {} (期望 ≈ 0.6)",
            predictor.beta_y
        );
        assert!(
            (predictor.beta_m - 0.4).abs() < 0.01,
            "beta_m = {} (期望 ≈ 0.4)",
            predictor.beta_m
        );
        assert_eq!(predictor.n_samples, 10);
    }

    /// 非有限或极大的观测: 返回 Err, 不 panic (服务端后台会拿历史实测调用 fit).
    #[test]
    fn test_fit_rejects_non_finite_and_overflowing_input() {
        let mut with_nan = make_obs(10);
        with_nan[3].csr_measured = f64::NAN;
        assert!(CsrPredictor::fit(&with_nan).is_err());

        let mut with_inf = make_obs(10);
        with_inf[0].g = f64::INFINITY;
        assert!(CsrPredictor::fit(&with_inf).is_err());

        // 有限但极大: 平方求和溢出成 inf/NaN.
        let mut huge = make_obs(10);
        huge[2].a = 1e200;
        huge[5].csr_measured = 1e200;
        assert!(CsrPredictor::fit(&huge).is_err());
    }

    #[test]
    fn test_gauss_jordan_reports_nan_instead_of_panicking() {
        let mut aug = [[0.0; 8]; 7];
        for (index, row) in aug.iter_mut().enumerate() {
            row[index] = 1.0;
        }
        aug[2][0] = f64::NAN;
        assert!(gauss_jordan(&mut aug).is_err());
    }

    #[test]
    fn test_fit_too_few_samples() {
        let obs = make_obs(5);
        let result = CsrPredictor::fit(&obs);
        assert!(result.is_err(), "5 个样本应返回 Err");
        let msg = result.unwrap_err();
        assert!(msg.contains("样本数不足"), "错误信息: {}", msg);
    }
}
