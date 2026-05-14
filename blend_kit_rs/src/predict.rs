//! 可选的 CSR 预测模块.
//!
//! 通过历史 [混合 S/A/V/G/Y/M, 实测 CSR] 数据拟合线性回归公式:
//!   CSR_predicted = β₀ + β_S·S + β_A·A + β_V·V + β_G·G + β_Y·Y + β_M·M
//!
//! 不破坏默认 per-coal CSR 直接录入行为, 仅在业务侧主动调用时生效.

/// 单次历史配煤观测记录.
#[derive(Debug, Clone)]
pub struct CsrObservation {
    pub s: f64,
    pub a: f64,
    pub v: f64,
    pub g: f64,
    pub y: f64,
    pub m: f64,
    pub csr_measured: f64,
}

/// 拟合后的 CSR 线性预测器.
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
            return Err(format!(
                "样本数不足: 需要至少 7 个观测, 实际 {}",
                n
            ));
        }

        // 构造设计矩阵 X (n×7) 和目标向量 y (n).
        // 列顺序: [1, S, A, V, G, Y, M]
        const P: usize = 7;
        let mut xt_x = [[0f64; P]; P]; // XᵀX, 7×7
        let mut xt_y = [0f64; P];      // Xᵀy, 7

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
            aug[0][P],
            aug[1][P],
            aug[2][P],
            aug[3][P],
            aug[4][P],
            aug[5][P],
            aug[6][P],
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

    /// 给定 6 个混合指标, 预测 CSR.
    pub fn predict(&self, s: f64, a: f64, v: f64, g: f64, y: f64, m: f64) -> f64 {
        self.intercept
            + self.beta_s * s
            + self.beta_a * a
            + self.beta_v * v
            + self.beta_g * g
            + self.beta_y * y
            + self.beta_m * m
    }

    /// 从 Coal 的 props 中取 6 个指标预测 CSR. 任意指标缺失返回 None.
    pub fn predict_coal(&self, coal: &crate::Coal) -> Option<f64> {
        Some(self.predict(
            coal.get("S")?,
            coal.get("A")?,
            coal.get("V")?,
            coal.get("G")?,
            coal.get("Y")?,
            coal.get("M")?,
        ))
    }
}

/// 对 7×8 增广矩阵做高斯-若尔当消元 (全主元), 求解 7 元线性方程组.
/// 就地修改, 解写回最后一列. 矩阵奇异时返回 Err.
fn gauss_jordan(aug: &mut [[f64; 8]; 7]) -> Result<(), String> {
    const N: usize = 7;
    for col in 0..N {
        // 选列最大主元 (部分主元).
        let pivot_row = (col..N)
            .max_by(|&a, &b| aug[a][col].abs().partial_cmp(&aug[b][col].abs()).unwrap());
        let pivot_row = pivot_row.unwrap();
        if aug[pivot_row][col].abs() < 1e-12 {
            return Err("矩阵奇异: 自变量之间存在完全共线性".into());
        }
        aug.swap(col, pivot_row);

        let pivot = aug[col][col];
        for j in col..=N {
            aug[col][j] /= pivot;
        }

        for row in 0..N {
            if row == col {
                continue;
            }
            let factor = aug[row][col];
            for j in col..=N {
                aug[row][j] -= factor * aug[col][j];
            }
        }
    }
    Ok(())
}

// ============================================================================
// 单元测试
// ============================================================================

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
                CsrObservation { s, a, v, g, y, m, csr_measured: csr }
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

    #[test]
    fn test_fit_too_few_samples() {
        let obs = make_obs(5);
        let result = CsrPredictor::fit(&obs);
        assert!(result.is_err(), "5 个样本应返回 Err");
        let msg = result.unwrap_err();
        assert!(msg.contains("样本数不足"), "错误信息: {}", msg);
    }

    #[test]
    fn test_predict_coal() {
        // 构造临北煤: (S=2.0, A=6.0, V=22, G=93, Y=17, petro=0.01, CSR=70, M=11, FOB=1425, FRT=25)
        let linbei = crate::coal_from_tuple(
            "临北",
            (2.0, 6.0, 22.0, 93.0, 17.0, 0.01, 70.0, 11.0, 1425.0, 25.0),
        );

        // 用一个已知系数构造 predictor, 验证 predict_coal 能拿到合理值.
        // 以 predict(2.0, 6.0, 22.0, 93.0, 17.0, 11.0) 为参考:
        // = 30 + 1*2 + 0.5*6 + 0.8*22 + 0.3*93 + 0.6*17 + 0.4*11
        // = 30 + 2 + 3 + 17.6 + 27.9 + 10.2 + 4.4 = 95.1
        let predictor = CsrPredictor {
            intercept: 30.0,
            beta_s: 1.0,
            beta_a: 0.5,
            beta_v: 0.8,
            beta_g: 0.3,
            beta_y: 0.6,
            beta_m: 0.4,
            r_squared: 0.999,
            n_samples: 10,
        };

        let predicted = predictor.predict_coal(&linbei);
        assert!(predicted.is_some(), "临北煤应该能预测 CSR");
        let v = predicted.unwrap();
        assert!(
            (v - 95.1).abs() < 0.01,
            "predicted CSR = {} (期望 ≈ 95.1)",
            v
        );

        // 测试缺少指标时返回 None
        let mut bad_coal = linbei.clone();
        bad_coal.props.remove("G");
        assert!(
            predictor.predict_coal(&bad_coal).is_none(),
            "缺 G 指标应返回 None"
        );
    }
}
