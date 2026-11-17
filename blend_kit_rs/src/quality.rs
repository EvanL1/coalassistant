//! 指标的业务判定和 LP 可表达式.
//!
//! 核心分离:
//! - `AcceptanceRule`: 合同如何判定报告值；
//! - `margin`: 内部留的安全余量, 只收紧 LP；
//! - `MetricFormula`: 建 LP 行与体检共用的表达式。

use crate::model::*;

/// 判定规则自己那一层的容限: 把"落在档位边界上"的浮点噪声吸掉 (`judged_value` 与
/// 合同界的贴格, 见 `snap_to_grid`)，以及 `raw_pass` —— 后者只区分"✓"和
/// "≈判定通过"，不决定这一单认不认。
///
/// **它不是"认不认"的容限**。认不认看 [`ACCEPT_TOLERANCE`]，那一个必须跟着 LP 走；
/// 这一个跟着判定档位走，两者本来就不是同一个量，别再并成一个数。
const CHECK_TOLERANCE: f64 = 1e-8;
const BINDING_TOL: f64 = 0.05;

/// 解后复核"这一单认不认"的相对容限。
///
/// **直接取 LP 那一个, 不另起一个数** —— 这里修的缺陷就是两个容限各说各话:
/// LP 用相对判据放行, 解后复核用绝对 `CHECK_TOLERANCE`(1e-8) 收口, 于是 Hard 界
/// 顶格时 LP 认下的最优解被自己的体检否掉, 可行的合同报成"约束冲突, LP 不可行"
/// (实测 A≤11.3 那一单落在执行界外 1.05e-8). 共用同一个常量, 两边就不会再分家。
///
/// 判据是 `距离 >= -本值 * (1 + 量级)`, 量级取 `max(|实测值|, |执行界|)`。
///
/// ## 实测记录 —— 全 crate 唯一真源
///
/// **这张表是这些数字在 crate 里的唯一出处。** `optimizer::FEASIBILITY_TOLERANCE`、
/// `lib.rs` 的拒收线测试等处只引用、不复述 —— 三份手抄的数字就是三个会各自变旧的
/// 地方, 这事已经发生过一次 (符号 bug 订正后, 另外两处还停在旧文案)。
///
/// 数字由 `optimizer::measurements::measure_tolerance_headroom` 产出, 可复跑:
///
/// ```text
/// cargo test --release -- --ignored measure_tolerance_headroom --nocapture
/// ```
///
/// 网格: master 煤池 (4 条 verified + 31 条化验齐全按品质合成价) 与用户线上那 4 条,
/// 五种合同变体 (裸合同 / margin=0.3 / 两位小数四舍五入 / 两位小数截断 / 上下双界),
/// 逐条 Hard 界按 0.01 步长扫过 ±200 档 (宽煤池 0.03 步长 ±120 档)。
///
/// 2026-09-27 重测 (判定改为"容限在量化之前加"之后): 样本数比上一版多了约 3.2 万条,
/// 多出来的是以前被判定掉格误杀、现在救回的解, 残差上限也随之变大。
///
/// | 口径 | 线性行 |
/// |---|---|
/// | 最大 `residual/(1+复核量级)` | **5.9871e-8** |
/// | └ 本值留出的余量 | 1.7 倍 |
/// | 最大 `residual/(1+LP 行量级)` | **9.6727e-8** |
/// | └ `FEASIBILITY_TOLERANCE` 留出的余量 | **1.03 倍** |
/// | 最小 `复核量级/LP 行量级` (顶界行) | **1.0716** |
/// | └ 同上, 全部带界行 | 0.925 |
/// | 最大绝对残差 (指标单位) | 1.5594e-7 |
/// | 复核侧放行量 (指标单位) | 9.40e-6 |
/// | LP 侧放行量 (指标单位) | 2.03e-6 |
/// | 残差样本 / 量级样本 | 11.8 万 / 29.7 万 |
///
/// 这张表原先还有"仿射/回归行"一列 (G 仿射校准、CSR 回归进 LP 时的行), 2026-11-17 随
/// 评估器一起删了: 求解只剩线性行.
///
/// 两个"放行量"**不是同一个量, 别互相抄**: 复核侧是 `本值 × (1 + max(|实测值|,
/// |执行界|))`, 量级取界本身 (灰分 ≈11); LP 侧是
/// `FEASIBILITY_TOLERANCE × (1 + Σ|aᵢxᵢ|)`, 量级取行系数相对绑定值的平均绝对偏差
/// (灰分行 ≈2)。两个常量相等, 两个放行量差着四五倍。
///
/// 对照化验 0.01 的分辨率: 复核侧最大 9.40e-6 ⇒ 约 **3 个数量级**;
/// LP 侧最大 2.03e-6 ⇒ 约 **3.7 个数量级**。合同界仍是硬墙。
///
/// ## "复核不会比 LP 严"不是恒等式
///
/// 两边共用一个常量, 但**量级口径不同**: 复核取 `max(|实测值|,|执行界|)`, LP 行取
/// `Σ|aᵢxᵢ|`。
///
/// 看表要认准"顶界行"那一行: 顶在界上、差一丝就被否掉的行, 最小比值 **1.0716**,
/// 复核没比 LP 严过, 余量 7%。全部带界行的最小值会跌到 0.92 以下, 那是**顶不到界**
/// 的行 —— 实测值离界很远, LP 行量级 `Σ|执行界−propᵢ|xᵢ` 随之变大, 比值自然小于 1。
/// 那种行的余量以"档"计, 容限差几倍毫无后果, 不是安全问题。
///
/// LP 行是 `formula_for` 的加权平均: `aᵢ = propᵢ − 执行界`, 最优解处
/// `Σ(propᵢ−界)xᵢ ≈ 0` ⇒ 正负偏差等量, 又因化验值非负, 负向偏差 ≤ 界 ⇒
/// `Σ|aᵢxᵢ| ≤ 2×界`。所以复核**最多比 LP 严 2 倍**。实测余量 1.7 倍**已盖不住**这个
/// 理论上界; 眼下的保障是实测 —— 顶界行上比值 ≥ 1.0716, 复核从没比 LP 严过。
/// 这是观测, 不是证明。
///
/// 这份实测记录是本值的唯一依据, 勿在没有重跑上面那条命令的情况下改动。
///
/// ## 容限必须在量化之前加 (2026-09-27 修复)
///
/// 截断/四舍五入是跳一整格的量化: 求解器残差把解推过格线一丝, 量化后就差一整格。
/// 原先容限加在量化**之后** (`judged + 放行量 >= 界`), 救不回那一格 —— 线上实例:
/// 两种煤、G≥80、一位小数截断, LP 解出 G=79.99999998857, 截断成 79.9, 判 Fail,
/// 有解的合同被报成"LP 不可行"。下界的执行界恰好压在格线上 (80.0), 残差一过线
/// 就发作; 上界执行界在 `下一格 − strict_bound_epsilon`, 残差把解推过执行界一丝时
/// 同样可能发作。
///
/// 现在 `check_value` 对原值判不过、又贴着执行界 (越界不超过放行量) 的值按方向先推
/// 再量化: 上界判 `judged(值 − 放行量)`, 下界判 `judged(值 + 放行量)`。外面还有一道
/// 闸 —— 离执行界不得超过放行量。这条后果链切断后, 抬本值不再会引入假不可行。
/// 原值本来就判得过的不推: 推了不改变结论, 只会让展示的判定值少一格
/// (`tests/judging_grid.rs` 的 `test_judged_value_of_an_in_bound_value_is_its_own_grid_cell`)。
///
/// ## 放行量不许盖过执行界留的缓冲
///
/// 那道外闸只有在"放行量 < 上界缓冲 [`strict_bound_epsilon`]"时才拦得住界外一格的值:
/// 上界执行界 = 进位线 − 缓冲, 压在进位线上的值离执行界正好一个缓冲。缓冲曾经只按格宽
/// 取 (两位小数 1e-6, 三位以上 1e-7), 而放行量随量级涨 (灰分 ≈10.5 时 1.15e-6),
/// 量级一过 9 就反超 —— A=10.51 对"≤10.50 截断两位"被判通过, 判定值写着 10.50;
/// 同一单走求解, LP 说不可行, 诊断用同一个放行量去量, 一项也指认不出来。
///
/// 两条规矩把这件事钉死 (见 [`strict_bound_epsilon`] 与 [`judging_allowance`]):
///
/// 1. 缓冲至少是进位线处放行量的 2 倍 —— 贴着执行界的解 (残差 ≤ 放行量) 照认,
///    压在进位线上的值 (离执行界一个缓冲) 必判 Fail, 两边各留一倍。
/// 2. 放行量不超过 1/16 格, 缓冲不超过 1/8 格 —— 格子细到与求解器残差同量级时
///    (五六位小数), 宁可让解后复核说"没算出可靠结论", 也不把界外一格的值认下来。
///
/// 于是对任何判定规则、任何合同界: 判定值越界的值 (界外一格及更远) 必判 Fail;
/// 顶在执行界上、只差求解器残差的解不判 Fail。
/// `test_one_grid_beyond_bound_always_fails` 与
/// `test_accept_tolerance_never_outruns_strict_bound_epsilon` 钉住这两头。
///
/// 没选的修法: 把下界执行界也往里缩一个 eps (与上界"对称")。上界的缓冲在格子
/// 远端 (2.5999), 碰不到常见的整数化验值; 下界的缓冲却在近端 (80.00001), 恰好
/// 卡掉 G 正好 80 的煤 —— 见 `tests/truncated_lower_bound.rs`。
///
/// **别"为了保险"把它调紧。** 线性 Hard 指标真正拦人的是 LP 那一行: 它的界就是
/// `effective_upper`/`effective_lower`, 与这里比的是同一条线, 所以越界的解根本
/// 走不到复核跟前 —— 复核是 LP 下游的冗余安全网, 不是墙。
///
/// 自己动手验: 把本值临时放大四个数量级, 重跑上面那条 `--ignored` 命令, 上表的
/// "最大绝对残差"纹丝不动 (仍是 1.56e-7)。拦人的从来是 LP。
/// 于是调紧只有一个效果: 把 LP 认下的正确解否掉。旧的绝对 1e-8 就是这样, 它从来
/// 没挡下过任何真越界, 只制造了"可行合同报不可行"。
pub(crate) const ACCEPT_TOLERANCE: f64 = crate::optimizer::FEASIBILITY_TOLERANCE;

/// 放行量 = `ACCEPT_TOLERANCE × (1 + 量级)`，量级取"实测值与界"里大的那个。
///
/// 全 crate "相对容限 × (1 + 量级)" 只有这一处; 只有一个量级时两个参数传同一个数。
pub(crate) fn accept_slack(value: f64, bound: f64) -> f64 {
    ACCEPT_TOLERANCE * (1.0 + value.abs().max(bound.abs()))
}

/// 放行量最多占一格的几分之几。
const MAX_ALLOWANCE_CELLS: f64 = 1.0 / 16.0;
/// 上界缓冲最多占一格的几分之几; 必须大于 [`MAX_ALLOWANCE_CELLS`]。
const MAX_EPSILON_CELLS: f64 = 1.0 / 8.0;

/// 判"值是否还贴着执行界"用的放行量: [`accept_slack`], 但不超过 1/16 格。
///
/// 体检 ([`check_value`]) 与不可行诊断共用 —— 两边用的不是同一个量时, 体检判 Fail
/// 的值诊断会说"没越界", 报出"约束冲突"却指认不出是哪一项。
pub(crate) fn judging_allowance(value: f64, executed: f64, rule: &AcceptanceRule) -> f64 {
    accept_slack(value, executed).min(quantum(rule) * MAX_ALLOWANCE_CELLS)
}

/// 一项指标按配比算出来的式子: `intercept + Σ分子·x / Σ分母·x`.
///
/// 现在八项指标都是按配比线性加权 ([`formula_for`]): `intercept` 恒为 0, 分母恒为 1.
/// 这两样留着不删, 是因为去掉它们会改变浮点运算的次序 (`0.0 + x`、`x / Σ配比`),
/// 结果的末位不保证不变.
#[derive(Debug, Clone)]
pub(crate) struct MetricFormula {
    /// 分子系数 (各煤的指标值; 煤阶交互会平移 CSR 这一行).
    pub numerators: Vec<f64>,
    /// 分母系数.
    pub denominators: Vec<f64>,
    pub intercept: f64,
    pub method: EvaluationMethod,
    pub verified: bool,
}

impl MetricFormula {
    pub fn linear(coefficients: Vec<f64>, method: EvaluationMethod, verified: bool) -> Self {
        let n = coefficients.len();
        Self {
            numerators: coefficients,
            denominators: vec![1.0; n],
            intercept: 0.0,
            method,
            verified,
        }
    }

    /// 简单加权值 `Σ分子·x` (不除以 Σ配比), 体检行的 `proxy_value`.
    pub fn proxy_value(&self, ratios: &[f64]) -> f64 {
        dot(&self.numerators, ratios)
    }

    pub fn evaluate(&self, ratios: &[f64]) -> Option<f64> {
        let denominator = dot(&self.denominators, ratios);
        if !denominator.is_finite() || denominator <= 1e-12 {
            return None;
        }
        Some(self.intercept + dot(&self.numerators, ratios) / denominator)
    }

    /// 把 `evaluate(x) <= upper` 编译为 `a·x <= b`.
    pub fn upper_constraint(&self, upper: f64) -> (Vec<f64>, f64) {
        let shifted = upper - self.intercept;
        (
            self.numerators
                .iter()
                .zip(&self.denominators)
                .map(|(numerator, denominator)| numerator - shifted * denominator)
                .collect(),
            0.0,
        )
    }

    /// 把 `evaluate(x) >= lower` 编译为 `a·x <= b`.
    pub fn lower_constraint(&self, lower: f64) -> (Vec<f64>, f64) {
        let shifted = lower - self.intercept;
        (
            self.numerators
                .iter()
                .zip(&self.denominators)
                .map(|(numerator, denominator)| shifted * denominator - numerator)
                .collect(),
            0.0,
        )
    }
}

fn dot(values: &[f64], ratios: &[f64]) -> f64 {
    values
        .iter()
        .zip(ratios)
        .map(|(value, ratio)| value * ratio)
        .sum()
}

pub(crate) fn acceptance_rule(spec: &Spec, legacy_truncate: bool) -> AcceptanceRule {
    spec.acceptance.clone().unwrap_or_else(|| {
        if legacy_truncate && spec.indicator != "petro" {
            AcceptanceRule {
                mode: AcceptanceMode::Truncate,
                decimals: Some(1),
                tolerance: 0.0,
            }
        } else {
            AcceptanceRule::default()
        }
    })
}

/// 判定规则的 `10^小数位`。全 crate 只在这里算一次; 外面要格宽用 [`quantum`]。
fn scale(rule: &AcceptanceRule) -> f64 {
    10_f64.powi(i32::from(rule.decimals.unwrap_or(0).min(6)))
}

/// 判定一格的宽度 (`10^-小数位`)。Raw 不量化, 这一格只用来给放行量封顶。
pub(crate) fn quantum(rule: &AcceptanceRule) -> f64 {
    1.0 / scale(rule)
}

/// 已乘过 `scale` 的值若贴在格线上 (差在 `CHECK_TOLERANCE` 以内) 就取那条格线。
///
/// 格点上的十进制数乘 `scale` 很少正好是整数: 0.29 × 100 = 28.999999999999996,
/// 0.07 × 100 = 7.000000000000001。不先吸掉这点误差就 floor/ceil, 会偏出整整一格。
fn snap_to_grid(scaled: f64, scale: f64) -> f64 {
    let nearest = scaled.round();
    if (scaled - nearest).abs() <= CHECK_TOLERANCE * scale {
        nearest
    } else {
        scaled
    }
}

/// 不超过 `value` 的最近一条格线。
pub(crate) fn grid_floor(value: f64, rule: &AcceptanceRule) -> f64 {
    let scale = scale(rule);
    snap_to_grid(value * scale, scale).floor() / scale
}

/// 不低于 `value` 的最近一条格线。
pub(crate) fn grid_ceil(value: f64, rule: &AcceptanceRule) -> f64 {
    let scale = scale(rule);
    snap_to_grid(value * scale, scale).ceil() / scale
}

/// 上界执行界离进位线 `line` 留的缓冲: 进位线上的值判定时已经进到下一格, LP 不许顶到。
///
/// 取"万分之一格"与"进位线处放行量的 2 倍"里大的, 再封顶到 1/8 格。第二项保证放行量
/// 盖不过缓冲 (见 [`ACCEPT_TOLERANCE`] 的"放行量不许盖过执行界留的缓冲"一节);
/// 一位小数、量级 49 以内时第一项 (1e-5) 更大, 默认合同的执行界与以前逐位相同。
fn strict_bound_epsilon(rule: &AcceptanceRule, line: f64) -> f64 {
    let quantum = quantum(rule);
    (quantum * 1e-4)
        .max(2.0 * accept_slack(line, line))
        .min(quantum * MAX_EPSILON_CELLS)
}

pub(crate) fn judged_value(value: f64, rule: &AcceptanceRule) -> f64 {
    let scale = scale(rule);
    let scaled = value * scale;
    let stabilized = match rule.mode {
        AcceptanceMode::Raw => scaled,
        AcceptanceMode::Truncate => snap_to_grid(scaled, scale),
        AcceptanceMode::Round => {
            let boundary = (scaled - 0.5).round() + 0.5;
            if (scaled - boundary).abs() <= CHECK_TOLERANCE * scale {
                boundary
            } else {
                scaled
            }
        }
    };
    match rule.mode {
        AcceptanceMode::Raw => value,
        AcceptanceMode::Truncate => stabilized.trunc() / scale,
        AcceptanceMode::Round => stabilized.round() / scale,
    }
}

/// 合同上限按判定规则折成的执行界: 判定值不超过 `bound + tolerance` 的最大原值,
/// 再往里留一个 [`strict_bound_epsilon`]。
fn effective_upper(bound: f64, rule: &AcceptanceRule) -> f64 {
    let accepted = bound + rule.tolerance.max(0.0);
    // 进位线: 原值到了这条线, 判定值就进到下一格.
    let line = match rule.mode {
        AcceptanceMode::Raw => return accepted,
        AcceptanceMode::Truncate => grid_floor(accepted, rule) + quantum(rule),
        AcceptanceMode::Round => grid_floor(accepted, rule) + quantum(rule) * 0.5,
    };
    line - strict_bound_epsilon(rule, line)
}

/// 合同下限按判定规则折成的执行界: 判定值不低于 `bound − tolerance` 的最小原值。
/// 不留缓冲 —— 正好压在线上的值判定时算达标。
fn effective_lower(bound: f64, rule: &AcceptanceRule) -> f64 {
    let accepted = bound - rule.tolerance.max(0.0);
    match rule.mode {
        AcceptanceMode::Raw => accepted,
        AcceptanceMode::Truncate => grid_ceil(accepted, rule),
        AcceptanceMode::Round => grid_ceil(accepted, rule) - quantum(rule) * 0.5,
    }
}

/// 上限在 LP 里真正执行的界 = 判定规则折算 − 安全余量。建 LP 行、计价块、诊断、
/// 反解放宽目标、验算扣款都从这里取, 不各写一遍。
pub(crate) fn executed_upper(bound: f64, rule: &AcceptanceRule, margin: f64) -> f64 {
    effective_upper(bound, rule) - margin
}

/// 下限在 LP 里真正执行的界 = 判定规则折算 + 安全余量。见 [`executed_upper`]。
pub(crate) fn executed_lower(bound: f64, rule: &AcceptanceRule, margin: f64) -> f64 {
    effective_lower(bound, rule) + margin
}

/// 判定值要比的那条线 (上界): 合同界 + 允许偏差, 非 Raw 规则下贴到不超过它的最近格线上。
///
/// 判定值都在格线上, 所以 "判定值 ≤ 界" 与 "判定值 ≤ 不超过界的最近格线" 是一回事。贴格是为了
/// 吸掉 界 ± 偏差 的浮点零头 (0.7 + 0.1 = 0.7999999999999999): LP 的执行界 ([`effective_upper`])
/// 用的就是这条贴过格的线, 判定这一侧不贴, LP 认的解这里就不认。
fn judged_limit_upper(bound: f64, rule: &AcceptanceRule) -> f64 {
    let accepted = bound + rule.tolerance.max(0.0);
    match rule.mode {
        AcceptanceMode::Raw => accepted,
        AcceptanceMode::Truncate | AcceptanceMode::Round => grid_floor(accepted, rule),
    }
}

/// 判定值要比的那条线 (下界): 合同界 − 允许偏差, 非 Raw 规则下贴到不低于它的最近格线上。
/// 见 [`judged_limit_upper`]。
fn judged_limit_lower(bound: f64, rule: &AcceptanceRule) -> f64 {
    let accepted = bound - rule.tolerance.max(0.0);
    match rule.mode {
        AcceptanceMode::Raw => accepted,
        AcceptanceMode::Truncate | AcceptanceMode::Round => grid_ceil(accepted, rule),
    }
}

pub(crate) struct CheckOutcome {
    pub judged: f64,
    pub slack: Option<f64>,
    pub binding: bool,
    pub status: EvaluationStatus,
}

pub(crate) fn check_value(
    value: f64,
    spec: Option<&Spec>,
    legacy_truncate: bool,
    verified: bool,
) -> CheckOutcome {
    let Some(spec) = spec else {
        return CheckOutcome {
            judged: value,
            slack: None,
            binding: false,
            status: if verified {
                EvaluationStatus::Pass
            } else {
                EvaluationStatus::Unverified
            },
        };
    };
    let rule = acceptance_rule(spec, legacy_truncate);
    let use_max = matches!(spec.direction, Direction::Upper | Direction::Range);
    let use_min = matches!(spec.direction, Direction::Lower | Direction::Range);
    let mut accepted_slacks = Vec::new();
    let mut raw_pass = true;
    let mut judged_pass = true;

    if use_max {
        if let Some(maximum) = spec.max {
            let executed = effective_upper(maximum, &rule);
            let allow = judging_allowance(value, executed, &rule);
            let distance = executed - value;
            let limit = judged_limit_upper(maximum, &rule);
            // 先按原值判. 原值判不过、而它离执行界不超过放行量时, 才当它是求解器残差,
            // 在量化之前往里推一个放行量再判: 截断/四舍五入是跳一整格的, 量化之后再加
            // 容限, 救不回已被残差推过格线的那一格.
            // 只在这一种情况下推. 界内的值照原值算 —— 不然正好压在格线上的值会被推到
            // 前一格 (灰分 10.30 显示成判定值 10.2); 离执行界更远的不是残差, 也照原值算
            // —— 不然判了 Fail, 展示的判定值却还在界内.
            let plain = judged_value(value, &rule);
            let lenient = if plain > limit && distance >= -allow {
                judged_value(value - allow, &rule)
            } else {
                plain
            };
            accepted_slacks.push((distance, allow, lenient));
            raw_pass &= value <= maximum + CHECK_TOLERANCE;
            judged_pass &= lenient <= limit;
        }
    }
    if use_min {
        if let Some(minimum) = spec.min {
            let executed = effective_lower(minimum, &rule);
            let allow = judging_allowance(value, executed, &rule);
            let distance = value - executed;
            let limit = judged_limit_lower(minimum, &rule);
            // 与上界同理: 原值判不过、且离执行界不超过放行量时才往里推.
            let plain = judged_value(value, &rule);
            let lenient = if plain < limit && distance >= -allow {
                judged_value(value + allow, &rule)
            } else {
                plain
            };
            accepted_slacks.push((distance, allow, lenient));
            raw_pass &= value + CHECK_TOLERANCE >= minimum;
            judged_pass &= lenient >= limit;
        }
    }

    // 上下界都在场时取最紧的那一侧, 连同它自己的放行量一起带走 —— 放行量随量级变化,
    // 只留 slack 会让下面拿另一侧的量级去判这一侧.
    //
    // 挑"最紧"用的是原始距离而非距离/放行量之比: 只有当下界的执行界高过上界的执行界
    // 时两者才会挑出不同的一侧, 而 `validate_request` 已经挡掉"下限大于上限", 所以
    // 这两种挑法在能走到这里的输入上等价.
    let tightest = accepted_slacks
        .into_iter()
        .reduce(|left, right| if right.0 < left.0 { right } else { left });
    let slack = tightest.map(|(distance, _, _)| distance);
    // 展示的判定值与做判定的是同一个数, 否则会出现"判定 10.01"旁边写"通过".
    let judged = tightest.map_or_else(|| judged_value(value, &rule), |(_, _, lenient)| lenient);
    let binding =
        tightest.is_some_and(|(distance, allow, _)| distance > -allow && distance < BINDING_TOL);
    let accepted = tightest.is_none_or(|(distance, allow, _)| distance >= -allow) && judged_pass;
    let status = if !accepted {
        EvaluationStatus::Fail
    } else if !verified {
        EvaluationStatus::Unverified
    } else if !raw_pass {
        EvaluationStatus::TolerancePass
    } else {
        EvaluationStatus::Pass
    };

    CheckOutcome {
        judged,
        slack,
        binding,
        status,
    }
}

pub(crate) fn formula_for(indicator: &str, coals: &[&Coal]) -> Option<MetricFormula> {
    let proxy: Vec<f64> = coals
        .iter()
        .map(|coal| coal.get(indicator))
        .collect::<Option<Vec<_>>>()?;

    let (method, verified) = match indicator {
        // 这三项在求解器中按质量加权，算法本身可精确复算。
        "S" | "A" | "M" => (EvaluationMethod::Linear, true),
        // 其余指标混合后并不严格线性, 加权值只是估算, 不冒充已验证的结果。
        "V" | "G" | "Y" | "petro" | "CSR" => (EvaluationMethod::ProvisionalLinear, false),
        _ => (EvaluationMethod::Unavailable, false),
    };
    Some(MetricFormula::linear(proxy, method, verified))
}

/// 单煤边界校验: 煤名非空、价格与指标为非负有限数、指标已知、采购条款合法.
/// 煤池与价格因子的 extra_coals 共用, 重名由调用方按各自范围判.
pub(crate) fn validate_coal(coal: &Coal) -> Result<(), String> {
    if coal.name.trim().is_empty() {
        return Err("煤名不能为空".into());
    }
    if !coal.fob.is_finite() || !coal.frt.is_finite() || coal.fob < 0.0 || coal.frt < 0.0 {
        return Err(format!("{} 的价格必须是非负有限数", coal.name));
    }
    if coal.props.iter().any(|(indicator, value)| {
        !INDICATORS.contains(&indicator.as_str()) || !value.is_finite() || *value < 0.0
    }) {
        return Err(format!("{} 含未知指标、负值或非法数值", coal.name));
    }
    if let Some(terms) = &coal.purchase_terms {
        validate_purchase_terms(&coal.name, terms)?;
    }
    Ok(())
}

/// 方向忽略的一侧 (Upper 的 min / Lower 的 max) 在入口处清掉, 之后全链路只看
/// 归一化值: 校验不查它、LP 不建它、体检不回显它、余量步长不读它.
/// 输入形状不变 —— 不清用户的请求对象, 返回的是内部副本; 无需清理时返回 None.
pub(crate) fn normalize_ignored_sides(request: &BlendRequest) -> Option<BlendRequest> {
    let ignored = |spec: &Spec| match spec.direction {
        Direction::Upper => spec.min.is_some(),
        Direction::Lower => spec.max.is_some(),
        Direction::Range => false,
    };
    if !request.specs.iter().any(ignored) {
        return None;
    }
    let mut normalized = request.clone();
    for spec in &mut normalized.specs {
        match spec.direction {
            Direction::Upper => spec.min = None,
            Direction::Lower => spec.max = None,
            Direction::Range => {}
        }
    }
    Some(normalized)
}

pub(crate) fn validate_request(request: &BlendRequest) -> Result<(), String> {
    if request.coals.is_empty() {
        return Err("煤池不能为空".into());
    }
    if request
        .total_quantity
        .is_some_and(|quantity| !quantity.is_finite() || quantity <= 0.0)
    {
        return Err("采购总量必须是正数".into());
    }
    // 一铲比采购量还大不算错: shovel::solve 提示后按连续比例求解.
    if request
        .shovel_tons
        .is_some_and(|tons| !tons.is_finite() || tons <= 0.0)
    {
        return Err("一铲吨数必须是正数".into());
    }

    let mut names = std::collections::HashSet::new();
    for coal in &request.coals {
        validate_coal(coal)?;
        // 煤池里的煤按自身报价直接进 LP, 不走估价 (估价只给 price_factor.extra_coals);
        // 出厂价 ≤ 0 按约定就是"没报价" (pricing / price_factor 同一约定), 这种煤进 LP
        // 会被当成近乎免费的煤把配方整个拽过去, 哪怕运费不是 0.
        if coal.fob <= 0.0 {
            return Err(format!(
                "{} 的出厂价为 0 (没报价), 不能当免费煤求解: 没报价的煤请按估价求解",
                coal.name
            ));
        }
        if !names.insert(coal.name.as_str()) {
            return Err(format!("煤名重复: {}", coal.name));
        }
    }
    for (name, floor) in &request.min_ratios {
        if !names.contains(name.as_str()) {
            return Err(format!("配比下限里的 {name} 不在煤池中"));
        }
        if !floor.is_finite() || !(0.0..=1.0).contains(floor) {
            return Err(format!("{name} 的配比下限 {floor} 须在 0~1 之间"));
        }
    }
    if request.min_ratios.values().sum::<f64>() > 1.0 + 1e-9 {
        return Err("配比下限合计超过 100%".into());
    }

    let mut indicators = std::collections::HashSet::new();
    for spec in request.specs.iter().filter(|item| item.enabled) {
        if !INDICATORS.contains(&spec.indicator.as_str()) {
            return Err(format!("未知合同指标: {}", spec.indicator));
        }
        if !indicators.insert(spec.indicator.as_str()) {
            return Err(format!("合同指标重复: {}", spec.indicator));
        }
        let needs_max = matches!(spec.direction, Direction::Upper | Direction::Range);
        let needs_min = matches!(spec.direction, Direction::Lower | Direction::Range);
        if needs_max && spec.max.is_none() {
            return Err(format!("{} 缺少上限", label_zh(&spec.indicator)));
        }
        if needs_min && spec.min.is_none() {
            return Err(format!("{} 缺少下限", label_zh(&spec.indicator)));
        }
        if spec
            .min
            .into_iter()
            .chain(spec.max)
            .any(|bound| !bound.is_finite())
        {
            return Err(format!(
                "{} 合同边界必须是有限数",
                label_zh(&spec.indicator)
            ));
        }
        // min > max 只有 Range 才查: Upper 的 min、Lower 的 max 被方向忽略,
        // 残留值不是错误. 入口归一化后它们已不在, 这里按方向再挡一层.
        if spec.direction == Direction::Range
            && spec
                .min
                .zip(spec.max)
                .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(format!("{} 下限大于上限", label_zh(&spec.indicator)));
        }
        if spec
            .margin
            .is_some_and(|margin| !margin.is_finite() || margin < 0.0)
        {
            return Err(format!("{} 安全余量非法", label_zh(&spec.indicator)));
        }
        if let Some(rule) = &spec.acceptance {
            if (rule.mode != AcceptanceMode::Raw && rule.decimals.is_none())
                || rule.decimals.is_some_and(|decimals| decimals > 6)
                || !rule.tolerance.is_finite()
                || rule.tolerance < 0.0
            {
                return Err(format!("{} 判定规则非法", label_zh(&spec.indicator)));
            }
        }
        if spec.enforcement == Enforcement::Priced {
            if spec.direction == Direction::Range {
                return Err(format!(
                    "{} 区间型指标不支持计价",
                    label_zh(&spec.indicator)
                ));
            }
            let Some(penalty) = &spec.penalty else {
                return Err(format!(
                    "{} 启用计价但缺扣款条款",
                    label_zh(&spec.indicator)
                ));
            };
            // Range 已在上面提前返回, 这里只剩 Upper/Lower; needs_max/needs_min 检查
            // (本函数前半段) 已保证对应边界存在, Some(..) else 只是防御式兜底.
            let bound = if spec.direction == Direction::Upper {
                spec.max
            } else {
                spec.min
            };
            let Some(bound) = bound else {
                return Err(format!("{} 计价缺少合同边界", label_zh(&spec.indicator)));
            };
            validate_penalty(&spec.indicator, spec.direction, bound, penalty)?;
        }
    }
    Ok(())
}

/// 校验单条采购合同计价条款 (买入侧). 规则与卖出侧 `validate_penalty` 共用,
/// 额外校验 guarantee 有限、指标已知且不重复、方向不为 Range, 以及水分折算参数的合理范围.
fn validate_purchase_terms(coal_name: &str, terms: &PurchaseTerms) -> Result<(), String> {
    let mut indicators = std::collections::HashSet::new();
    for clause in &terms.clauses {
        if !INDICATORS.contains(&clause.indicator.as_str()) {
            return Err(format!(
                "{} 采购条款指标未知: {}",
                coal_name, clause.indicator
            ));
        }
        if !indicators.insert(clause.indicator.as_str()) {
            // 同一指标两条条款会在 effective_cif 里被各自的扣款重复计算,
            // 让煤价被低估, LP 因此过量买入该煤 (同类问题参见凸性检查).
            return Err(format!(
                "{} 采购条款指标重复: {}",
                coal_name, clause.indicator
            ));
        }
        if clause.direction == Direction::Range {
            return Err(format!(
                "{} {} 区间型指标不支持采购计价",
                coal_name,
                label_zh(&clause.indicator)
            ));
        }
        if !clause.guarantee.is_finite() {
            return Err(format!(
                "{} {} 采购保证值必须是有限数",
                coal_name,
                label_zh(&clause.indicator)
            ));
        }
        validate_penalty(
            &clause.indicator,
            clause.direction,
            clause.guarantee,
            &clause.penalty,
        )
        .map_err(|err| format!("{coal_name} {err}"))?;
    }
    // 真实合同对水分二选一: 扣量(结算量 = 净重 ×(1−M实)/(1−M合))或扣价(元/吨).
    // 两者同时配置会让每吨水分被扣两遍 —— 折价与折量各算一次, 账面数字仍然合理,
    // 不会有任何报错. 全局模板一旦同时带上, 整个煤池被系统性低估.
    if terms.contract_moisture.is_some()
        && terms.clauses.iter().any(|clause| clause.indicator == "M")
    {
        return Err(format!(
            "{coal_name} 水分不能同时按扣量和扣价计: 合同水分(contract_moisture)是扣量, \
             水分条款是扣价, 两种机制互斥, 请只保留一种"
        ));
    }
    if terms
        .contract_moisture
        .is_some_and(|moisture| !moisture.is_finite() || !(0.0..=100.0).contains(&moisture))
    {
        return Err(format!("{coal_name} 合同水分必须在 0~100 之间"));
    }
    if terms
        .moisture_excess_double_threshold
        .is_some_and(|threshold| !threshold.is_finite() || !(0.0..=100.0).contains(&threshold))
    {
        return Err(format!("{coal_name} 水分双倍阈值必须在 0~100 之间"));
    }
    Ok(())
}

/// 校验计价条款. 凸性(rate 递增)与拒收线方向是安全性的核心:
/// 前者错会让 LP 低估扣款, 后者错会让计价区间为空.
///
/// `bound` 是调用方按方向解出的合同边界 (Upper 传 max, Lower 传 min);
/// 两处调用方都已在调用前把 Range 拒掉, 也都已保证 bound 存在.
fn validate_penalty(
    indicator: &str,
    direction: Direction,
    bound: f64,
    penalty: &Penalty,
) -> Result<(), String> {
    let label = label_zh(indicator);
    if penalty.tiers.is_empty() {
        return Err(format!("{label} 扣款档位不能为空"));
    }
    if !penalty.reject.is_finite() {
        return Err(format!("{label} 拒收线必须是有限数"));
    }

    let last = penalty.tiers.len() - 1;
    let mut previous_rate = f64::NEG_INFINITY;
    for (index, tier) in penalty.tiers.iter().enumerate() {
        if !tier.rate.is_finite() {
            return Err(format!("{label} 第 {} 档扣款率必须是有限数", index + 1));
        }
        if tier.rate < 0.0 {
            return Err(format!(
                "{label} 第 {} 档扣款率不能为负 (rate={})",
                index + 1,
                tier.rate
            ));
        }
        if tier.rate <= previous_rate {
            return Err(format!(
                "{label} 第 {} 档扣款率未高于前一档: 档位必须递增才能保证凸性",
                index + 1
            ));
        }
        previous_rate = tier.rate;

        if index == last {
            if tier.width.is_some() {
                return Err(format!("{label} 末档不能有宽度上限"));
            }
        } else {
            match tier.width {
                Some(width) if width.is_finite() && width > 0.0 => {}
                _ => return Err(format!("{label} 第 {} 档宽度必须是正数", index + 1)),
            }
        }
    }

    match direction {
        Direction::Upper => {
            if penalty.reject < bound {
                return Err(format!(
                    "{label} 拒收线必须不低于合同上限 (拒收线 {} < 上限 {})",
                    penalty.reject, bound
                ));
            }
        }
        Direction::Lower => {
            if penalty.reject > bound {
                return Err(format!(
                    "{label} 拒收线必须不高于合同下限 (拒收线 {} > 下限 {})",
                    penalty.reject, bound
                ));
            }
        }
        Direction::Range => return Err(format!("{label} 区间型指标不支持计价")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 放行量永远盖不过 `strict_bound_epsilon`: 顶在上界执行界、只差求解器残差的解要认,
    /// 压在进位线上 (判定值已是界外一格) 的值必须判 Fail.
    ///
    /// 见 [`ACCEPT_TOLERANCE`] 的"放行量不许盖过执行界留的缓冲"一节. 界放在 10.00、
    /// 两位小数截断: 进位线 10.01, 放行量 ≈1.1e-6. 缓冲曾经只有 1e-6, 比放行量小,
    /// 10.01 这个值被外闸放行、再被"先推再量化"吸回 10.00 那一格, 判成通过.
    ///
    /// 这条测试以前叫 `test_accept_tolerance_can_outrun_strict_bound_epsilon`, 断言
    /// "放行量 > 缓冲"是前提, 并断言缝里的值 (执行界外 (缓冲+放行量)/2) 判定值 10.00、
    /// 不 Fail. 那个前提本身就是缺陷: 缝里的值离进位线只差 5e-8, 进位线上的值同样
    /// 落在放行量以内. 现在断言相反的不变量, 并把两头都钉住.
    /// 直接喂 `check_value` 构造值, 不经求解器.
    #[test]
    fn test_accept_tolerance_never_outruns_strict_bound_epsilon() {
        let mut spec = Spec::upper("A", 10.00);
        spec.acceptance = Some(AcceptanceRule {
            mode: AcceptanceMode::Truncate,
            decimals: Some(2),
            tolerance: 0.0,
        });
        let rule = acceptance_rule(&spec, false);

        let line = 10.01;
        let epsilon = strict_bound_epsilon(&rule, line);
        let executed = effective_upper(10.00, &rule);
        let allowance = judging_allowance(line, executed, &rule);
        assert!(
            allowance * 2.0 <= epsilon,
            "放行量至多是缓冲的一半: 放行量 {allowance:e} vs 缓冲 {epsilon:e}"
        );

        // 顶在执行界上、残差用满放行量的解: 认, 判定值留在合同那一格.
        let outcome = check_value(executed + allowance * 0.99, Some(&spec), false, true);
        assert_ne!(outcome.status, EvaluationStatus::Fail, "求解器残差要认");
        assert_eq!(outcome.judged, 10.00, "判定值留在合同那一格");

        // 压在进位线上的值: 截断后是 10.01, 超了合同, 判定值也要如实写 10.01.
        let outcome = check_value(line, Some(&spec), false, true);
        assert_eq!(outcome.status, EvaluationStatus::Fail, "界外一格必须 Fail");
        assert_eq!(outcome.judged, 10.01, "判 Fail 时展示的判定值不能还在界内");
    }
    /// 带 "允许偏差" 的界: 界 ± 偏差算出来常带浮点零头 (0.7161 + 0.1 = 0.8160999999999999,
    /// 17.44 − 0.7 = 16.740000000000002)。LP 的执行界是贴过格的, 判定这一侧也得贴 —— 否则
    /// LP 认的解体检不认, 可行的合同被报成 "没算出结论"。
    #[test]
    fn test_tolerance_bound_with_float_fuzz_is_judged_on_the_grid() {
        let status = |value: f64, spec: &Spec| check_value(value, Some(spec), false, true).status;
        let mut fuzzy = 0;
        for mode in [AcceptanceMode::Truncate, AcceptanceMode::Round] {
            for decimals in 1..=4u8 {
                for tolerance in [0.01, 0.05, 0.1, 0.5, 0.7] {
                    let rule = AcceptanceRule {
                        mode,
                        decimals: Some(decimals),
                        tolerance,
                    };
                    let scale = scale(&rule);
                    let cells = tolerance * scale;
                    if (cells - cells.round()).abs() > 1e-9 {
                        continue; // 偏差比判定的一格还细: 不在这条测试的范围.
                    }
                    let cells = cells.round();
                    for base in [
                        0.07, 0.29, 0.7161, 0.849, 1.13, 2.5, 10.49, 17.44, 28.0, 80.0, 99.9,
                    ] {
                        let grid = (base * scale).round();
                        let bound = grid / scale;
                        let label = format!("{mode:?} {decimals} 位, 界 {bound} ± {tolerance}");
                        // 合同认可的最后一格 (整数算出来的, 没有零头) 与它外面一格.
                        let last_up = (grid + cells) / scale;
                        let last_down = (grid - cells) / scale;
                        if bound + tolerance != last_up || bound - tolerance != last_down {
                            fuzzy += 1;
                        }

                        let mut upper = Spec::upper("A", bound);
                        upper.acceptance = Some(rule.clone());
                        let mut lower = Spec::lower("A", bound);
                        lower.acceptance = Some(rule.clone());

                        assert_ne!(
                            status(last_up, &upper),
                            EvaluationStatus::Fail,
                            "{label}: 压在上界 + 偏差上 ({last_up})"
                        );
                        assert_ne!(
                            status(effective_upper(bound, &rule), &upper),
                            EvaluationStatus::Fail,
                            "{label}: 顶在上界执行界"
                        );
                        assert_eq!(
                            status((grid + cells + 1.0) / scale, &upper),
                            EvaluationStatus::Fail,
                            "{label}: 上界 + 偏差外一格"
                        );
                        if last_down > 0.0 {
                            assert_ne!(
                                status(last_down, &lower),
                                EvaluationStatus::Fail,
                                "{label}: 压在下界 − 偏差上 ({last_down})"
                            );
                            assert_ne!(
                                status(effective_lower(bound, &rule), &lower),
                                EvaluationStatus::Fail,
                                "{label}: 顶在下界执行界"
                            );
                            assert_eq!(
                                status((grid - cells - 1.0) / scale, &lower),
                                EvaluationStatus::Fail,
                                "{label}: 下界 − 偏差外一格"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            fuzzy > 50,
            "这条测试要扫到带浮点零头的界, 实际只有 {fuzzy} 个"
        );
    }

    /// 判定格线: 合同界外一格 (及更远) 的值, 任何判定规则下都必须判 Fail; 正好压在
    /// 合同界上、或顶在执行界上的值不许判 Fail.
    ///
    /// 界取在各自的格点上 (化验与合同都按格报数), 量级从 0.0x 到 100 都扫 —— 放行量随
    /// 量级变大, 缺陷正是出在"量级大、格子细"的组合上.
    #[test]
    fn test_one_grid_beyond_bound_always_fails() {
        let status = |value: f64, spec: &Spec| check_value(value, Some(spec), false, true).status;
        for mode in [
            AcceptanceMode::Raw,
            AcceptanceMode::Truncate,
            AcceptanceMode::Round,
        ] {
            for decimals in 0..=6u8 {
                let rule = AcceptanceRule {
                    mode,
                    decimals: Some(decimals),
                    tolerance: 0.0,
                };
                let scale = scale(&rule);
                for magnitude in [0.07, 0.29, 0.849, 1.13, 2.5, 10.49, 10.5, 28.0, 80.0, 99.9] {
                    let grid = (magnitude * scale).round();
                    let bound = grid / scale;
                    let label = format!("{mode:?} {decimals} 位, 界 {bound}");

                    let mut upper = Spec::upper("A", bound);
                    upper.acceptance = Some(rule.clone());
                    let mut lower = Spec::lower("A", bound);
                    lower.acceptance = Some(rule.clone());

                    assert_ne!(
                        status(bound, &upper),
                        EvaluationStatus::Fail,
                        "{label}: 上界上"
                    );
                    assert_ne!(
                        status(bound, &lower),
                        EvaluationStatus::Fail,
                        "{label}: 下界上"
                    );
                    assert_ne!(
                        status(effective_upper(bound, &rule), &upper),
                        EvaluationStatus::Fail,
                        "{label}: 顶在上界执行界"
                    );
                    assert_ne!(
                        status(effective_lower(bound, &rule), &lower),
                        EvaluationStatus::Fail,
                        "{label}: 顶在下界执行界"
                    );
                    for steps in [1.0, 2.0, 3.0, 10.0] {
                        let above = (grid + steps) / scale;
                        assert_eq!(
                            status(above, &upper),
                            EvaluationStatus::Fail,
                            "{label}: 上界外 {steps} 格 ({above})"
                        );
                        let below = (grid - steps) / scale;
                        assert_eq!(
                            status(below, &lower),
                            EvaluationStatus::Fail,
                            "{label}: 下界外 {steps} 格 ({below})"
                        );
                    }
                }
            }
        }
    }

    /// 合同界本身落在格点上时, `界 × 10^位数` 的浮点误差不许让执行界偏一整格.
    ///
    /// 0.29 × 100 = 28.999999999999996, 直接 floor 得 28, 上界执行界就落到 0.29 以下 ——
    /// 合同写 ≤0.29, 化验 0.295 (截断后正是 0.29) 却被挡在外面. 0.07 × 100 =
    /// 7.000000000000001, 直接 ceil 得 8, 下界执行界成了 0.08.
    #[test]
    fn test_grid_bounds_keep_their_own_cell() {
        for decimals in 1..=6u8 {
            let scale = 10_f64.powi(i32::from(decimals));
            let quantum = 1.0 / scale;
            let truncate = AcceptanceRule {
                mode: AcceptanceMode::Truncate,
                decimals: Some(decimals),
                tolerance: 0.0,
            };
            let round = AcceptanceRule {
                mode: AcceptanceMode::Round,
                ..truncate.clone()
            };
            for grid in 1..=3000u32 {
                let bound = f64::from(grid) / scale;
                let upper = effective_upper(bound, &truncate);
                assert!(
                    upper > bound + quantum * 0.5 && upper < bound + quantum,
                    "截断 {decimals} 位, 上界 {bound}: 执行界 {upper} 应在这一格的后半格"
                );
                let lower = effective_lower(bound, &truncate);
                assert!(
                    (lower - bound).abs() < quantum * 1e-6,
                    "截断 {decimals} 位, 下界 {bound}: 执行界 {lower} 应就是这条格线"
                );
                let upper = effective_upper(bound, &round);
                assert!(
                    upper > bound && upper < bound + quantum * 0.5,
                    "四舍五入 {decimals} 位, 上界 {bound}: 执行界 {upper} 应在进位线内侧"
                );
                let lower = effective_lower(bound, &round);
                assert!(
                    (lower - (bound - quantum * 0.5)).abs() < quantum * 1e-6,
                    "四舍五入 {decimals} 位, 下界 {bound}: 执行界 {lower} 应是进位线"
                );
            }
        }
    }
    /// 计价条款校验的共享用例 (`data/penalty_cases.json`).
    ///
    /// 同一份用例前端也跑一遍 (`doudou_blend/src/penalty.test.ts`): 前端那套
    /// 字段级提示是本函数规则的镜像, 两份实现各自演化就会出现"界面放行、求解
    /// 报错"或反过来. 用例是它们不许分叉的证据 —— 改规则时先往这份数据里加
    /// 一条, 两端会同时红.
    #[derive(serde::Deserialize)]
    struct PenaltyCase {
        name: String,
        direction: Direction,
        bound: f64,
        penalty: Penalty,
        valid: bool,
    }

    #[derive(serde::Deserialize)]
    struct PenaltyCases {
        cases: Vec<PenaltyCase>,
    }

    #[test]
    fn shared_penalty_cases_match_fixture() {
        let raw = include_str!("../data/penalty_cases.json");
        let fixture: PenaltyCases = serde_json::from_str(raw).expect("用例文件必须是合法 JSON");
        assert!(fixture.cases.len() >= 10, "用例太少, 覆盖不到每条规则");

        for case in &fixture.cases {
            let result = validate_penalty("A", case.direction, case.bound, &case.penalty);
            assert_eq!(
                result.is_ok(),
                case.valid,
                "用例「{}」: 期望 valid={}, 实际 {:?}",
                case.name,
                case.valid,
                result
            );
        }
    }

    #[test]
    fn test_truncate_upper_boundary() {
        let rule = AcceptanceRule {
            mode: AcceptanceMode::Truncate,
            decimals: Some(1),
            tolerance: 0.0,
        };
        assert!((judged_value(2.5999, &rule) - 2.5).abs() < 1e-9);
        assert!((judged_value(79.999_999_995, &rule) - 80.0).abs() < 1e-9);
        assert!((judged_value(79.999_9, &rule) - 79.9).abs() < 1e-9);
        assert!(effective_upper(2.5, &rule) < 2.6);
        assert!(effective_upper(2.5, &rule) > 2.599_9);
    }

    #[test]
    fn test_round_lower_boundary() {
        let rule = AcceptanceRule {
            mode: AcceptanceMode::Round,
            decimals: Some(1),
            tolerance: 0.0,
        };
        assert!((effective_lower(80.0, &rule) - 79.95).abs() < 1e-9);
        assert_eq!(judged_value(79.96, &rule), 80.0);
    }

    /// 求解器残差会把解推到执行界外一丝; 截断/四舍五入是跳一整格的量化, 所以容限
    /// 必须在量化**之前**按方向推 —— 在量化之后加容限救不回已经掉下去的一格.
    /// 原先就是后者: G≥80 的解 79.99999998857 截断成 79.9, 整单被判"LP 不可行".
    ///
    /// 残差按各行自己的放行量取比例 (放行量随量级变, S≈2.6 的行只有 3.6e-7, G≈80 的行
    /// 有 8.1e-6), 外加线上那个实测值 1.143e-8. 真超标的值必须照样判 Fail ——
    /// 容限只吸收求解器噪声, 不放宽合同.
    #[test]
    fn test_judging_absorbs_solver_residual_but_not_real_violations() {
        for mode in [AcceptanceMode::Truncate, AcceptanceMode::Round] {
            let rule = AcceptanceRule {
                mode,
                decimals: Some(1),
                tolerance: 0.0,
            };
            let mut lower = Spec::lower("G", 80.0);
            lower.acceptance = Some(rule.clone());
            let mut upper = Spec::upper("S", 2.5);
            upper.acceptance = Some(rule.clone());
            let status =
                |value: f64, spec: &Spec| check_value(value, Some(spec), false, true).status;

            let low_edge = effective_lower(80.0, &rule);
            let high_edge = effective_upper(2.5, &rule);
            for fraction in [0.0, 0.1, 0.5, 0.99] {
                let residual = fraction * accept_slack(low_edge, low_edge);
                let low = low_edge - residual;
                assert_ne!(
                    status(low, &lower),
                    EvaluationStatus::Fail,
                    "{mode:?} 下界 残差 {residual:e}"
                );
                let residual = fraction * accept_slack(high_edge, high_edge);
                let high = high_edge + residual;
                assert_ne!(
                    status(high, &upper),
                    EvaluationStatus::Fail,
                    "{mode:?} 上界 残差 {residual:e}"
                );
            }
            // 线上那一单的原值: 截断下 G 79.99999998857 曾被判成 79.9.
            if mode == AcceptanceMode::Truncate {
                let outcome = check_value(80.0 - 1.143e-8, Some(&lower), false, true);
                assert_ne!(outcome.status, EvaluationStatus::Fail);
                assert_eq!(outcome.judged, 80.0, "展示的判定值要和做判定的一致");
            }
            let below = effective_lower(80.0, &rule) - 0.01;
            assert_eq!(
                status(below, &lower),
                EvaluationStatus::Fail,
                "{mode:?} 真低于下界"
            );
            let above = effective_upper(2.5, &rule) + 0.01;
            assert_eq!(
                status(above, &upper),
                EvaluationStatus::Fail,
                "{mode:?} 真高于上界"
            );
        }
    }

    fn priced_spec(tiers: Vec<PenaltyTier>, reject: f64) -> Spec {
        Spec {
            indicator: "A".into(),
            direction: Direction::Upper,
            min: None,
            max: Some(10.0),
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Priced,
            penalty: Some(Penalty { tiers, reject }),
        }
    }

    fn request_with(spec: Spec) -> BlendRequest {
        let mut props = std::collections::HashMap::new();
        for indicator in INDICATORS {
            props.insert(indicator.to_string(), 1.0);
        }
        BlendRequest {
            coals: vec![Coal {
                name: "甲".into(),
                props,
                fob: 1000.0,
                frt: 0.0,
                purchase_terms: None,
                coal_type: None,
                form: None,
                region: None,
                province: None,
                city: None,
                county: None,
            }],
            specs: vec![spec],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        }
    }

    /// 方向忽略的一侧必须当作不存在: Upper 带一个比上限还大的残留 min 不再被拒,
    /// Lower 带一个比下限还小的残留 max 不再被拒; 只有 Range 才查 min > max.
    #[test]
    fn test_ignored_side_is_not_validated() {
        let mut upper = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            12.0,
        );
        upper.min = Some(99.0);
        assert!(
            validate_request(&request_with(upper)).is_ok(),
            "Upper 的 min 被忽略, 不该查 min > max"
        );

        let mut lower = Spec::lower("G", 80.0);
        lower.max = Some(10.0);
        assert!(
            validate_request(&request_with(lower)).is_ok(),
            "Lower 的 max 被忽略, 不该查 min > max"
        );

        let mut range = Spec::upper("A", 10.0);
        range.direction = Direction::Range;
        range.min = Some(12.0);
        let err = validate_request(&request_with(range)).expect_err("Range 仍要查 min > max");
        assert!(err.contains("下限大于上限"), "{err}");
    }
    #[test]
    fn test_priced_spec_requires_penalty() {
        let mut spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            12.0,
        );
        spec.penalty = None;
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "Priced 缺 penalty 应报错"
        );
    }

    #[test]
    fn test_penalty_rates_must_increase() {
        // 递减 rate 破坏凸性: LP 会填错档并低估扣款
        let spec = priced_spec(
            vec![
                PenaltyTier {
                    width: Some(0.5),
                    rate: 80.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 40.0,
                },
            ],
            12.0,
        );
        let err = validate_request(&request_with(spec)).expect_err("rate 递减应报错");
        assert!(err.contains("递增"), "错误信息应提及递增, 实际: {err}");
    }

    #[test]
    fn test_penalty_reject_must_be_outside_contract_bound() {
        // Upper 向 reject 落在合同界内侧 ⇒ 计价区间为空
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            9.0,
        );
        let err = validate_request(&request_with(spec)).expect_err("reject 在合同界内侧应报错");
        assert!(err.contains("拒收线"), "错误信息应提及拒收线, 实际: {err}");
    }

    #[test]
    fn test_penalty_tiers_cannot_be_empty() {
        let spec = priced_spec(vec![], 12.0);
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "空档位应报错"
        );
    }

    #[test]
    fn test_penalty_reject_must_be_finite() {
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            f64::NAN,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "拒收线为 NaN 应报错"
        );
    }

    #[test]
    fn test_penalty_tier_rate_must_be_finite_and_non_negative() {
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: -1.0,
            }],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "负扣款率应报错"
        );
    }

    #[test]
    fn test_penalty_tail_tier_must_be_unbounded() {
        let spec = priced_spec(
            vec![PenaltyTier {
                width: Some(1.0),
                rate: 80.0,
            }],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "末档必须无上限"
        );
    }

    #[test]
    fn test_priced_spec_rejects_range_direction() {
        let mut spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            12.0,
        );
        spec.direction = Direction::Range;
        spec.min = Some(5.0);
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "Range 计价 spec 应报错"
        );
    }

    #[test]
    fn test_valid_priced_spec_passes() {
        let spec = priced_spec(
            vec![
                PenaltyTier {
                    width: Some(0.5),
                    rate: 10.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 20.0,
                },
            ],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_ok(),
            "合法计价条款应通过"
        );
    }

    #[test]
    fn test_valid_priced_spec_single_tier_passes() {
        // 单档 (width: None) 是真实合同最常见形态, 例如"每超 0.1% 扣 8 元/吨"无第二档.
        let spec = priced_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            12.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_ok(),
            "单档计价条款应通过"
        );
    }

    fn priced_lower_spec(tiers: Vec<PenaltyTier>, reject: f64) -> Spec {
        Spec {
            indicator: "G".into(),
            direction: Direction::Lower,
            min: Some(10.0),
            max: None,
            enabled: true,
            margin: None,
            acceptance: None,
            enforcement: Enforcement::Priced,
            penalty: Some(Penalty { tiers, reject }),
        }
    }

    #[test]
    fn test_penalty_lower_direction_reject_must_be_outside_contract_bound() {
        // Lower 向 reject 必须不高于合同下限, 否则计价区间为空
        let spec = priced_lower_spec(
            vec![PenaltyTier {
                width: None,
                rate: 80.0,
            }],
            11.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_err(),
            "Lower 向 reject 落在合同界内侧应报错"
        );
    }

    #[test]
    fn test_valid_priced_lower_spec_passes() {
        let spec = priced_lower_spec(
            vec![
                PenaltyTier {
                    width: Some(0.5),
                    rate: 10.0,
                },
                PenaltyTier {
                    width: None,
                    rate: 20.0,
                },
            ],
            8.0,
        );
        assert!(
            validate_request(&request_with(spec)).is_ok(),
            "合法 Lower 向计价条款应通过"
        );
    }

    fn coal_with_terms(terms: PurchaseTerms) -> Coal {
        let mut props = std::collections::HashMap::new();
        for indicator in INDICATORS {
            props.insert(indicator.to_string(), 1.0);
        }
        Coal {
            name: "乙".into(),
            props,
            fob: 1000.0,
            frt: 0.0,
            purchase_terms: Some(terms),
            coal_type: None,
            form: None,
            region: None,
            province: None,
            city: None,
            county: None,
        }
    }

    fn request_with_purchase_terms(terms: PurchaseTerms) -> BlendRequest {
        BlendRequest {
            coals: vec![coal_with_terms(terms)],
            specs: vec![],
            total_quantity: None,
            truncate_decimal: false,
            fixed_ratios: None,
            rank_interaction: None,
            price_factor: None,
            price_robustness: None,
            shovel_tons: None,
            min_ratios: Default::default(),
        }
    }

    #[test]
    fn test_purchase_clause_rejects_range_direction() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Range,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧 Range 应报错"
        );
    }

    #[test]
    fn test_purchase_clause_rejects_unknown_indicator() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "X".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧未知指标应报错"
        );
    }

    #[test]
    fn test_purchase_clause_reject_wrong_side_errors() {
        // Upper 向: reject 必须不低于保证值, 否则计价区间为空
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 9.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧 reject 落在保证值内侧应报错"
        );
    }

    #[test]
    fn test_purchase_clause_requires_finite_guarantee() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: f64::NAN,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧保证值必须是有限数"
        );
    }

    #[test]
    fn test_contract_moisture_out_of_range_errors() {
        let terms = PurchaseTerms {
            clauses: vec![],
            contract_moisture: Some(150.0),
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "合同水分超出合理范围应报错"
        );
    }

    #[test]
    fn test_moisture_excess_double_threshold_out_of_range_errors() {
        let terms = PurchaseTerms {
            clauses: vec![],
            contract_moisture: None,
            moisture_excess_double_threshold: Some(-1.0),
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "水分双倍阈值超出合理范围应报错"
        );
    }

    #[test]
    fn test_purchase_clause_rejects_duplicate_indicator() {
        // 同一指标两条条款会在 effective_cif 里被重复扣款, 低估该煤成本, LP 因此过量买入.
        let terms = PurchaseTerms {
            clauses: vec![
                PurchaseClause {
                    indicator: "A".into(),
                    direction: Direction::Upper,
                    guarantee: 10.0,
                    penalty: Penalty {
                        tiers: vec![PenaltyTier {
                            width: None,
                            rate: 80.0,
                        }],
                        reject: 12.0,
                    },
                },
                PurchaseClause {
                    indicator: "A".into(),
                    direction: Direction::Upper,
                    guarantee: 10.0,
                    penalty: Penalty {
                        tiers: vec![PenaltyTier {
                            width: None,
                            rate: 50.0,
                        }],
                        reject: 12.0,
                    },
                },
            ],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_err(),
            "买入侧同一指标重复应报错"
        );
    }

    #[test]
    fn test_valid_purchase_clause_single_tier_passes() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 80.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "单档买入侧计价条款应通过"
        );
    }

    #[test]
    fn test_valid_purchase_clause_passes() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty {
                    tiers: vec![
                        PenaltyTier {
                            width: Some(0.5),
                            rate: 10.0,
                        },
                        PenaltyTier {
                            width: None,
                            rate: 20.0,
                        },
                    ],
                    reject: 12.0,
                },
            }],
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: Some(10.0),
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "合法买入侧条款应通过"
        );
    }

    /// 水分两种机制互斥: contract_moisture 是扣量(折结算量), M 条款是扣价(元/吨).
    /// 全局模板若同时带上两者, 每个煤都会被水分扣两遍, 且账面完全合理、不会报错.
    #[test]
    fn test_moisture_quantity_and_price_mechanisms_are_exclusive() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "M".into(),
                direction: Direction::Upper,
                guarantee: 8.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 50.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: None,
        };
        let error = validate_request(&request_with_purchase_terms(terms))
            .expect_err("扣量与扣价同时配置应被拒绝");
        assert!(
            error.contains("乙") && error.contains("水分"),
            "报错应点名煤种与水分, 实得 {error}"
        );
    }

    /// 只用扣量(结算量折算)——我们样本合同的写法——应通过.
    #[test]
    fn test_moisture_quantity_mechanism_alone_passes() {
        let terms = PurchaseTerms {
            clauses: Vec::new(),
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: Some(12.0),
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "单用扣量机制应通过"
        );
    }

    /// 只用扣价(元/吨 水分条款)应通过.
    #[test]
    fn test_moisture_price_mechanism_alone_passes() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "M".into(),
                direction: Direction::Upper,
                guarantee: 8.0,
                penalty: Penalty {
                    tiers: vec![PenaltyTier {
                        width: None,
                        rate: 50.0,
                    }],
                    reject: 12.0,
                },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        assert!(
            validate_request(&request_with_purchase_terms(terms)).is_ok(),
            "单用扣价机制应通过"
        );
    }
}
