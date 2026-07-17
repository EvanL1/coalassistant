//! 可选的 CSR 预测模块.
//!
//! 通过历史 [六项线性代理快照, 实测 CSR] 数据拟合线性回归公式:
//!   CSR_predicted = β₀ + β_S·S + β_A·A + β_V·V + β_G·G + β_Y·Y + β_M·M
//!
//! 默认仍使用录入 CSR 代理；只有去重样本、LOOCV 误差与训练域均达标时，
//! 回归值才可进入混合层约束。

use serde::{Deserialize, Serialize};

use crate::{GObservation, ModelKind, ModelPolicy, ModelSummary};

const CSR_ALGORITHM_VERSION: &str = "csr-ridge-standardized-loocv-v2";
const G_ALGORITHM_VERSION: &str = "g-affine-loocv-v1";

/// 单次历史配煤观测记录. 六项特征必须是保存方案时的质量加权线性代理，
/// 不能逐字段混入事后实测值或 G 校准值。
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
            return Err(format!("样本数不足: 需要至少 7 个观测, 实际 {}", n));
        }

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

#[derive(Debug, Clone)]
pub(crate) struct ValidatedCsrModel {
    pub predictor: CsrPredictor,
    pub version: String,
    pub cv_mae: f64,
    pub p90_abs_error: f64,
    pub bias: f64,
    pub training_min: [f64; 6],
    pub training_max: [f64; 6],
}

impl ValidatedCsrModel {
    pub fn fit(observations: &[CsrObservation], policy: &ModelPolicy) -> Result<Self, String> {
        let unique_observations = deduplicate_csr_observations(observations);
        let observations = unique_observations.as_slice();
        if observations.len() < policy.min_csr_samples {
            return Err(format!(
                "去重后样本数不足: CSR 模型要求至少 {}, 实际 {}",
                policy.min_csr_samples,
                observations.len()
            ));
        }
        if observations.len() < 8 {
            return Err("样本数不足: 留一交叉验证至少需要 8 条 CSR 观测".into());
        }
        validate_csr_observations(observations)?;

        let predictor = fit_ridge(observations, policy.ridge_lambda)?;
        let mut residuals = Vec::with_capacity(observations.len());
        for leave_out in 0..observations.len() {
            let training: Vec<CsrObservation> = observations
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != leave_out)
                .map(|(_, observation)| observation.clone())
                .collect();
            let fold = fit_ridge(&training, policy.ridge_lambda)?;
            let observation = &observations[leave_out];
            residuals.push(
                fold.predict(
                    observation.s,
                    observation.a,
                    observation.v,
                    observation.g,
                    observation.y,
                    observation.m,
                ) - observation.csr_measured,
            );
        }
        let (cv_mae, p90_abs_error, bias) = residual_metrics(&residuals);
        let (training_min, training_max) = csr_feature_range(observations);
        let version = version_for_csr(
            observations,
            policy,
            &predictor,
            cv_mae,
            p90_abs_error,
            bias,
            training_min,
            training_max,
        );
        Ok(Self {
            predictor,
            version,
            cv_mae,
            p90_abs_error,
            bias,
            training_min,
            training_max,
        })
    }

    pub fn passes_gate(&self, policy: &ModelPolicy) -> bool {
        self.cv_mae <= policy.max_csr_cv_mae
    }

    pub fn is_in_domain(&self, features: [f64; 6], expansion: f64) -> bool {
        features
            .iter()
            .zip(self.training_min.iter().zip(&self.training_max))
            .all(|(value, (minimum, maximum))| {
                let width = (maximum - minimum).max(1e-9);
                *value >= minimum - width * expansion && *value <= maximum + width * expansion
            })
    }

    pub fn summary(&self, in_domain: bool) -> ModelSummary {
        ModelSummary {
            version: self.version.clone(),
            kind: ModelKind::CsrRidge,
            sample_count: self.predictor.n_samples,
            cv_mae: self.cv_mae,
            p90_abs_error: self.p90_abs_error,
            bias: self.bias,
            in_domain,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GAffinePredictor {
    pub intercept: f64,
    pub slope: f64,
    pub sample_count: usize,
}

impl GAffinePredictor {
    fn fit(observations: &[GObservation]) -> Result<Self, String> {
        if observations.len() < 2 {
            return Err("G 仿射模型至少需要 2 条观测".into());
        }
        if observations.iter().any(|observation| {
            !observation.g_linear.is_finite() || !observation.g_measured.is_finite()
        }) {
            return Err("G 观测包含非法数值".into());
        }
        let count = observations.len() as f64;
        let mean_x = observations
            .iter()
            .map(|observation| observation.g_linear)
            .sum::<f64>()
            / count;
        let mean_y = observations
            .iter()
            .map(|observation| observation.g_measured)
            .sum::<f64>()
            / count;
        let variance = observations
            .iter()
            .map(|observation| (observation.g_linear - mean_x).powi(2))
            .sum::<f64>();
        if variance <= 1e-12 {
            return Err("G 观测的线性代理没有变化，无法校准".into());
        }
        let covariance = observations
            .iter()
            .map(|observation| (observation.g_linear - mean_x) * (observation.g_measured - mean_y))
            .sum::<f64>();
        let slope = covariance / variance;
        if !slope.is_finite() || slope <= 0.0 {
            return Err("G 校准斜率必须为正".into());
        }
        Ok(Self {
            intercept: mean_y - slope * mean_x,
            slope,
            sample_count: observations.len(),
        })
    }

    pub fn predict(&self, g_linear: f64) -> f64 {
        self.intercept + self.slope * g_linear
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedGModel {
    pub predictor: GAffinePredictor,
    pub version: String,
    pub cv_mae: f64,
    pub p90_abs_error: f64,
    pub bias: f64,
    pub training_min: f64,
    pub training_max: f64,
}

impl ValidatedGModel {
    pub fn fit(observations: &[GObservation], policy: &ModelPolicy) -> Result<Self, String> {
        let unique_observations = deduplicate_g_observations(observations);
        let observations = unique_observations.as_slice();
        if observations.len() < policy.min_g_samples {
            return Err(format!(
                "去重后样本数不足: G 模型要求至少 {}, 实际 {}",
                policy.min_g_samples,
                observations.len()
            ));
        }
        if observations.len() < 3 {
            return Err("样本数不足: 留一交叉验证至少需要 3 条 G 观测".into());
        }
        let predictor = GAffinePredictor::fit(observations)?;
        let mut residuals = Vec::with_capacity(observations.len());
        for leave_out in 0..observations.len() {
            let training: Vec<GObservation> = observations
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != leave_out)
                .map(|(_, observation)| observation.clone())
                .collect();
            let fold = GAffinePredictor::fit(&training)?;
            let observation = &observations[leave_out];
            residuals.push(fold.predict(observation.g_linear) - observation.g_measured);
        }
        let (cv_mae, p90_abs_error, bias) = residual_metrics(&residuals);
        let training_min = observations
            .iter()
            .map(|observation| observation.g_linear)
            .fold(f64::INFINITY, f64::min);
        let training_max = observations
            .iter()
            .map(|observation| observation.g_linear)
            .fold(f64::NEG_INFINITY, f64::max);
        let version = version_for_g(
            observations,
            policy,
            &predictor,
            cv_mae,
            p90_abs_error,
            bias,
            training_min,
            training_max,
        );
        Ok(Self {
            predictor,
            version,
            cv_mae,
            p90_abs_error,
            bias,
            training_min,
            training_max,
        })
    }

    pub fn passes_gate(&self, policy: &ModelPolicy) -> bool {
        self.cv_mae <= policy.max_g_cv_mae
    }

    pub fn is_in_domain(&self, value: f64, expansion: f64) -> bool {
        let width = (self.training_max - self.training_min).max(1e-9);
        value >= self.training_min - width * expansion
            && value <= self.training_max + width * expansion
    }

    pub fn summary(&self, in_domain: bool) -> ModelSummary {
        ModelSummary {
            version: self.version.clone(),
            kind: ModelKind::GAffine,
            sample_count: self.predictor.sample_count,
            cv_mae: self.cv_mae,
            p90_abs_error: self.p90_abs_error,
            bias: self.bias,
            in_domain,
        }
    }
}

/// 求解阶段可选使用的已训练评估器.
///
/// 训练与求解刻意分开：调用方决定样本来自哪里，并在需要时显式调用 `train`；
/// 基础 `solve` 不读取样本，也不会在每次求解时重训。
#[derive(Debug, Clone, Default)]
pub struct EvaluatorSet {
    pub(crate) g: Option<ValidatedGModel>,
    pub(crate) csr: Option<ValidatedCsrModel>,
    pub(crate) warnings: Vec<String>,
    pub(crate) extrapolation_ratio: f64,
}

impl EvaluatorSet {
    /// 训练并门控 G/CSR 评估器。单个模型不达门槛时仅跳过该模型并保留警告。
    pub fn train(
        g_observations: &[GObservation],
        csr_observations: &[CsrObservation],
        policy: &ModelPolicy,
    ) -> Result<Self, String> {
        validate_model_policy(policy)?;
        let mut evaluators = Self {
            extrapolation_ratio: policy.extrapolation_ratio,
            ..Self::default()
        };

        if !g_observations.is_empty() {
            match ValidatedGModel::fit(g_observations, policy) {
                Ok(model) if model.passes_gate(policy) => evaluators.g = Some(model),
                Ok(model) => evaluators.warnings.push(format!(
                    "G 校准未启用: 交叉验证 MAE={:.2} > {:.2}",
                    model.cv_mae, policy.max_g_cv_mae
                )),
                Err(reason) => evaluators.warnings.push(format!("G 校准未启用: {reason}")),
            }
        }

        if !csr_observations.is_empty() {
            match ValidatedCsrModel::fit(csr_observations, policy) {
                Ok(model) if model.passes_gate(policy) => evaluators.csr = Some(model),
                Ok(model) => evaluators.warnings.push(format!(
                    "CSR 模型未启用: 交叉验证 MAE={:.2} > {:.2}",
                    model.cv_mae, policy.max_csr_cv_mae
                )),
                Err(reason) => evaluators
                    .warnings
                    .push(format!("CSR 模型未启用: {reason}")),
            }
        }

        Ok(evaluators)
    }
}

fn validate_model_policy(policy: &ModelPolicy) -> Result<(), String> {
    if policy.min_g_samples < 3
        || policy.min_csr_samples < 8
        || !policy.max_g_cv_mae.is_finite()
        || policy.max_g_cv_mae <= 0.0
        || !policy.max_csr_cv_mae.is_finite()
        || policy.max_csr_cv_mae <= 0.0
        || !policy.extrapolation_ratio.is_finite()
        || policy.extrapolation_ratio < 0.0
        || !policy.ridge_lambda.is_finite()
        || policy.ridge_lambda < 0.0
    {
        return Err("模型策略参数非法".into());
    }
    Ok(())
}

fn deduplicate_csr_observations(observations: &[CsrObservation]) -> Vec<CsrObservation> {
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<_> = observations
        .iter()
        .filter(|observation| {
            seen.insert([
                canonical_f64_bits(observation.s),
                canonical_f64_bits(observation.a),
                canonical_f64_bits(observation.v),
                canonical_f64_bits(observation.g),
                canonical_f64_bits(observation.y),
                canonical_f64_bits(observation.m),
                canonical_f64_bits(observation.csr_measured),
            ])
        })
        .cloned()
        .collect();
    unique.sort_by_key(|observation| {
        [
            canonical_f64_bits(observation.s),
            canonical_f64_bits(observation.a),
            canonical_f64_bits(observation.v),
            canonical_f64_bits(observation.g),
            canonical_f64_bits(observation.y),
            canonical_f64_bits(observation.m),
            canonical_f64_bits(observation.csr_measured),
        ]
    });
    unique
}

fn deduplicate_g_observations(observations: &[GObservation]) -> Vec<GObservation> {
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<_> = observations
        .iter()
        .filter(|observation| {
            seen.insert([
                canonical_f64_bits(observation.g_linear),
                canonical_f64_bits(observation.g_measured),
            ])
        })
        .cloned()
        .collect();
    unique.sort_by_key(|observation| {
        [
            canonical_f64_bits(observation.g_linear),
            canonical_f64_bits(observation.g_measured),
        ]
    });
    unique
}

fn fit_ridge(observations: &[CsrObservation], lambda: f64) -> Result<CsrPredictor, String> {
    if observations.len() < 7 {
        return Err(format!(
            "样本数不足: 需要至少 7 个观测, 实际 {}",
            observations.len()
        ));
    }
    validate_csr_observations(observations)?;
    const P: usize = 7;
    let count = observations.len() as f64;
    let mut means = [0.0; 6];
    for observation in observations {
        for (mean, value) in means.iter_mut().zip(csr_features(observation)) {
            *mean += value / count;
        }
    }
    let mut stddev = [0.0; 6];
    for observation in observations {
        for ((variance, value), mean) in stddev.iter_mut().zip(csr_features(observation)).zip(means)
        {
            *variance += (value - mean).powi(2) / count;
        }
    }
    for value in &mut stddev {
        *value = value.sqrt();
        if *value < 1e-9 {
            *value = 1.0;
        }
    }

    let mut xt_x = [[0.0; P]; P];
    let mut xt_y = [0.0; P];
    for observation in observations {
        let features = csr_features(observation);
        let mut row = [0.0; P];
        row[0] = 1.0;
        for index in 0..6 {
            row[index + 1] = (features[index] - means[index]) / stddev[index];
        }
        for row_index in 0..P {
            xt_y[row_index] += row[row_index] * observation.csr_measured;
            for column_index in 0..P {
                xt_x[row_index][column_index] += row[row_index] * row[column_index];
            }
        }
    }
    for (index, diagonal) in xt_x.iter_mut().enumerate().skip(1) {
        diagonal[index] += lambda;
    }
    let mut augmented = [[0.0; P + 1]; P];
    for row in 0..P {
        for column in 0..P {
            augmented[row][column] = xt_x[row][column];
        }
        augmented[row][P] = xt_y[row];
    }
    gauss_jordan(&mut augmented)?;
    let standardized = [
        augmented[0][P],
        augmented[1][P],
        augmented[2][P],
        augmented[3][P],
        augmented[4][P],
        augmented[5][P],
        augmented[6][P],
    ];
    let mut beta = [0.0; 6];
    for index in 0..6 {
        beta[index] = standardized[index + 1] / stddev[index];
    }
    let intercept = standardized[0]
        - beta
            .iter()
            .zip(means)
            .map(|(coefficient, mean)| coefficient * mean)
            .sum::<f64>();
    let y_mean = observations
        .iter()
        .map(|observation| observation.csr_measured)
        .sum::<f64>()
        / count;
    let ss_total = observations
        .iter()
        .map(|observation| (observation.csr_measured - y_mean).powi(2))
        .sum::<f64>();
    let predictor = CsrPredictor {
        intercept,
        beta_s: beta[0],
        beta_a: beta[1],
        beta_v: beta[2],
        beta_g: beta[3],
        beta_y: beta[4],
        beta_m: beta[5],
        r_squared: 0.0,
        n_samples: observations.len(),
    };
    let ss_residual = observations
        .iter()
        .map(|observation| {
            (predictor.predict(
                observation.s,
                observation.a,
                observation.v,
                observation.g,
                observation.y,
                observation.m,
            ) - observation.csr_measured)
                .powi(2)
        })
        .sum::<f64>();
    Ok(CsrPredictor {
        r_squared: if ss_total < 1e-12 {
            1.0
        } else {
            1.0 - ss_residual / ss_total
        },
        ..predictor
    })
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

fn csr_feature_range(observations: &[CsrObservation]) -> ([f64; 6], [f64; 6]) {
    let mut minimum = [f64::INFINITY; 6];
    let mut maximum = [f64::NEG_INFINITY; 6];
    for observation in observations {
        for (index, value) in csr_features(observation).into_iter().enumerate() {
            minimum[index] = minimum[index].min(value);
            maximum[index] = maximum[index].max(value);
        }
    }
    (minimum, maximum)
}

fn residual_metrics(residuals: &[f64]) -> (f64, f64, f64) {
    let count = residuals.len() as f64;
    let cv_mae = residuals.iter().map(|residual| residual.abs()).sum::<f64>() / count;
    let bias = residuals.iter().sum::<f64>() / count;
    let mut absolute: Vec<f64> = residuals.iter().map(|residual| residual.abs()).collect();
    absolute.sort_by(f64::total_cmp);
    let p90_index = ((absolute.len() as f64 * 0.9).ceil() as usize)
        .saturating_sub(1)
        .min(absolute.len() - 1);
    (cv_mae, absolute[p90_index], bias)
}

#[allow(clippy::too_many_arguments)]
fn version_for_csr(
    observations: &[CsrObservation],
    policy: &ModelPolicy,
    predictor: &CsrPredictor,
    cv_mae: f64,
    p90_abs_error: f64,
    bias: f64,
    training_min: [f64; 6],
    training_max: [f64; 6],
) -> String {
    let mut hash = FNV_OFFSET;
    hash_str(&mut hash, CSR_ALGORITHM_VERSION);
    hash_policy(&mut hash, policy);
    let mut rows: Vec<[u64; 7]> = observations
        .iter()
        .map(|observation| {
            [
                canonical_f64_bits(observation.s),
                canonical_f64_bits(observation.a),
                canonical_f64_bits(observation.v),
                canonical_f64_bits(observation.g),
                canonical_f64_bits(observation.y),
                canonical_f64_bits(observation.m),
                canonical_f64_bits(observation.csr_measured),
            ]
        })
        .collect();
    rows.sort_unstable();
    for row in rows {
        for bits in row {
            hash_u64(&mut hash, bits);
        }
    }
    for value in [
        predictor.intercept,
        predictor.beta_s,
        predictor.beta_a,
        predictor.beta_v,
        predictor.beta_g,
        predictor.beta_y,
        predictor.beta_m,
        cv_mae,
        p90_abs_error,
        bias,
    ]
    .into_iter()
    .chain(training_min)
    .chain(training_max)
    {
        hash_f64(&mut hash, value);
    }
    format!("csr-ridge-v2-{hash:016x}")
}

#[allow(clippy::too_many_arguments)]
fn version_for_g(
    observations: &[GObservation],
    policy: &ModelPolicy,
    predictor: &GAffinePredictor,
    cv_mae: f64,
    p90_abs_error: f64,
    bias: f64,
    training_min: f64,
    training_max: f64,
) -> String {
    let mut hash = FNV_OFFSET;
    hash_str(&mut hash, G_ALGORITHM_VERSION);
    hash_policy(&mut hash, policy);
    let mut rows: Vec<[u64; 2]> = observations
        .iter()
        .map(|observation| {
            [
                canonical_f64_bits(observation.g_linear),
                canonical_f64_bits(observation.g_measured),
            ]
        })
        .collect();
    rows.sort_unstable();
    for row in rows {
        for bits in row {
            hash_u64(&mut hash, bits);
        }
    }
    for value in [
        predictor.intercept,
        predictor.slope,
        cv_mae,
        p90_abs_error,
        bias,
        training_min,
        training_max,
    ] {
        hash_f64(&mut hash, value);
    }
    format!("g-affine-v1-{hash:016x}")
}

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn hash_f64(hash: &mut u64, value: f64) {
    hash_u64(hash, canonical_f64_bits(value));
}

fn canonical_f64_bits(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}

fn hash_u64(hash: &mut u64, value: u64) {
    for byte in value.to_le_bytes() {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn hash_str(hash: &mut u64, value: &str) {
    hash_u64(hash, value.len() as u64);
    for byte in value.bytes() {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn hash_policy(hash: &mut u64, policy: &ModelPolicy) {
    hash_u64(hash, policy.min_g_samples as u64);
    hash_u64(hash, policy.min_csr_samples as u64);
    hash_f64(hash, policy.max_g_cv_mae);
    hash_f64(hash, policy.max_csr_cv_mae);
    hash_f64(hash, policy.extrapolation_ratio);
    hash_f64(hash, policy.ridge_lambda);
}

/// 对 7×8 增广矩阵做高斯-若尔当消元 (全主元), 求解 7 元线性方程组.
/// 就地修改, 解写回最后一列. 矩阵奇异时返回 Err.
fn gauss_jordan(aug: &mut [[f64; 8]; 7]) -> Result<(), String> {
    const N: usize = 7;
    for col in 0..N {
        // 选列最大主元 (部分主元).
        let pivot_row =
            (col..N).max_by(|&a, &b| aug[a][col].abs().partial_cmp(&aug[b][col].abs()).unwrap());
        let pivot_row = pivot_row.unwrap();
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

    #[test]
    fn test_validated_versions_are_order_independent() {
        let policy = ModelPolicy {
            min_g_samples: 3,
            min_csr_samples: 8,
            max_g_cv_mae: 1.0,
            max_csr_cv_mae: 1.0,
            ..ModelPolicy::default()
        };
        let csr = make_obs(10);
        let mut reversed_csr = csr.clone();
        reversed_csr.reverse();
        let first = ValidatedCsrModel::fit(&csr, &policy).unwrap();
        let second = ValidatedCsrModel::fit(&reversed_csr, &policy).unwrap();
        assert_eq!(first.version, second.version);

        let g: Vec<GObservation> = (0..10)
            .map(|index| {
                let linear = 70.0 + f64::from(index);
                GObservation {
                    g_linear: linear,
                    g_measured: 1.0 + 0.9 * linear,
                }
            })
            .collect();
        let mut reversed_g = g.clone();
        reversed_g.reverse();
        let first = ValidatedGModel::fit(&g, &policy).unwrap();
        let second = ValidatedGModel::fit(&reversed_g, &policy).unwrap();
        assert_eq!(first.version, second.version);
    }

    #[test]
    fn test_duplicate_rows_do_not_satisfy_minimum_sample_gate() {
        let policy = ModelPolicy {
            min_g_samples: 3,
            min_csr_samples: 8,
            ..ModelPolicy::default()
        };
        let duplicate_csr = vec![make_obs(8)[0].clone(); 8];
        assert!(ValidatedCsrModel::fit(&duplicate_csr, &policy).is_err());
        let duplicate_g = vec![
            GObservation {
                g_linear: 80.0,
                g_measured: 72.0,
            };
            3
        ];
        assert!(ValidatedGModel::fit(&duplicate_g, &policy).is_err());
    }

    #[test]
    fn test_evaluator_policy_rejects_invalid_thresholds() {
        let policy = ModelPolicy {
            min_g_samples: 2,
            ..ModelPolicy::default()
        };
        assert!(EvaluatorSet::train(&[], &[], &policy).is_err());
    }
}
