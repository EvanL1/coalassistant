# 合同扣款条款进入配煤最优化模型 设计

日期: 2026-09-18
状态: 已批准（用户确认：两侧都做 / 全局模板+单煤覆盖 / 默认全硬+逐项开启计价）
来源: 用户提供的《煤炭买卖合同》BMY-XGX-ZJM-20260911（山西鑫高 → 北京天立，主焦煤 5000 吨）

## 目的

当前 LP 把合同质量要求当成硬墙：`min Σ cif·x` s.t. `a·x ≤ b`，违约成本隐含为 +∞。
真实合同不是这样写的——它给每项指标标了明码：灰分超标 80 元/吨·%，粘结不足只要 5 元/吨·点。
硬约束等价于"违约成本无穷大"，合同却说是 5 元。把 ∞ 换成 5，可行域和最优解都会变。

本设计把扣款条款引入模型，使"最优解"从**最便宜的合规配方**变成**净成本最低的配方**
（净成本 = 到厂价 − 买入侧扣款折扣 + 卖出侧质量扣款）。

## 合同条款的数学形式

合同以"每超 0.1% 扣 8 元"的形式书写，换算成每 1 个指标单位的斜率：

| 指标 | 合同界 | 一档斜率 | 一档宽度 | 二档斜率 |
|---|---|---|---|---|
| 灰 A | ≤10% | 80 元/吨·% | 无限 | — |
| 挥发 V | ≤28% | 10 元/吨·% | 0.5% | 20 元/吨·% |
| 硫 S | ≤1% | 200 元/吨·% | 0.05% | 400 元/吨·% |
| 胶质 Y | ≥15 | 10 元/吨·mm | 无限 | — |
| 粘结 G | ≥85 | 5 元/吨·点 | 无限 | — |
| 热强 CSR | ≥68 | 10 元/吨·点 | 5 点 | 20 元/吨·点 |
| 水分 M | ≤8% | 量折算，非价格扣款 | — | >12% 超出部分双倍 |

**凸性是本设计成立的前提。** 三个分档条款的斜率都递增（10→20、200→400、10→20），
即扣款函数是凸分段线性函数，可以用档位变量精确表达为 LP，无需整数变量。
若某档斜率递减（罚款打折），凸性破坏，LP 会静默填错档并低估扣款——因此
rate 递增必须是**硬校验**，不是警告。

### 条款歧义（待与对方确认，不阻塞实现）

1. "挥发超 0.1%，扣 1 元/吨"省略了"每"字。对照下一句"每超 0.1% 扣 2 元"，
   按"每 0.1% 扣 1 元"（=10 元/吨·%）实现。两种读法差一个量级。
2. 起扣点进位规则（超 0.05% 算 0 档还是按 0.1% 进位）未明确。
   本期不做台阶量化，见"不在范围内"。

## 两侧的符号方向（最易写反之处）

| | 触发 | 对现金流的影响 | 数学地位 |
|---|---|---|---|
| **买入侧** | 采购的单煤质量低于其采购合同保证值 | 供应商扣款 → **少付钱** → 成本**下降**（折扣） | 由该煤自身化验值决定，**常数**，折进 `c[i]` |
| **卖出侧** | 配出的混煤指标低于销售合同要求 | 买方扣款 → **少收钱** → 等价成本**上升** | 由混煤指标决定，是决策变量的函数，**必须进 LP** |

统一目标函数：

```
cif_eff_i = fob_i · (1 − M_eff_i)/(1 − M合_i) + frt_i − Σ_k 买入扣款_k(coal_i)

minimize   Σ_i cif_eff_i · x_i  +  Σ_k rate_k · d_k
subject to Σ_i x_i = 1,  x ≥ 0,  d ≥ 0
```

`frt` 不参与水分折算——**湿煤货款打折但运费照付**，水分是纯运费损失。

有效水分（合同"超 12% 超出部分双倍扣除水分"）：

```
M_eff = if M > threshold { 2·M − threshold } else { M }
```

`threshold` 为 None 时不启用双倍，`M_eff = M`。仍是凸分段线性。

## A. 数据模型（`blend_kit_rs/src/model.rs`）

```rust
/// 扣款档位: 从上一档终点起, 覆盖 width 宽度的偏离, 按 rate 计费.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PenaltyTier {
    /// 本档覆盖的偏离宽度(指标单位). None = 末档, 无上限.
    pub width: Option<f64>,
    /// 元/吨 per 1 个指标单位. 合同"每 0.1% 扣 8 元" → 80.0
    pub rate: f64,
}

/// 单项指标的计价条款.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Penalty {
    /// 档位表. 至少一档, rate 严格递增(凸性), width > 0.
    pub tiers: Vec<PenaltyTier>,
    /// 拒收线: 越过即硬不可行. Upper 向是上限, Lower 向是下限.
    pub reject: f64,
}
```

### 卖出侧：挂在 `Spec` 上

```rust
pub enum Enforcement {
    #[default]
    Hard,
    Soft,
    Advisory,
    /// 进入 LP, 但超界不判不可行, 而是按 Penalty 折算成元/吨计入目标;
    /// 越过 Penalty.reject 仍然硬不可行.
    Priced,
}

pub struct Spec {
    // ... 既有字段不变 ...
    /// enforcement == Priced 时必填.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub penalty: Option<Penalty>,
}
```

### 买入侧：挂在 `Coal` 上

```rust
/// 单条采购合同计价条款.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PurchaseClause {
    pub indicator: String,
    /// 仅接受 Upper(超标扣) / Lower(低于扣); Range 校验报错.
    pub direction: Direction,
    /// 该煤采购合同的保证值.
    pub guarantee: f64,
    pub penalty: Penalty,
}

/// 采购合同条款. 前端已把全局模板与单煤覆盖合并完毕后传入.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PurchaseTerms {
    pub clauses: Vec<PurchaseClause>,
    /// 合同水分, 用于结算量折算. None = 不做折算.
    pub contract_moisture: Option<f64>,
    /// 超过该水分时超出部分双倍折算. None = 不启用.
    pub moisture_double_threshold: Option<f64>,
}

pub struct Coal {
    // ... 既有字段不变 ...
    /// None = 按报价原值, 不做买入侧修正.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase_terms: Option<PurchaseTerms>,
}
```

**全局模板与单煤覆盖的合并在前端完成，core 只收合并后的结果。**
与仓库既有分层一致（Master 只读、用户改动走 `storage.ts` / `cloudStorage.ts`）。

### margin 的作用域

`Priced` 指标上，`margin` **只收紧拒收线，不收紧合同界**。
合同界是白纸黑字的精确数字，提前开扣会系统性高估扣款；拒收线是风险判断，该留安全垫。

## B. 校验（`blend_kit_rs/src/quality.rs::validate_request`）

以下全部为**硬校验**（返回 `Err`，不是 warning）：

1. `Enforcement::Priced` 的 Spec 必须有 `penalty`
2. `Penalty.tiers` 非空
3. `tiers` 的 `rate` 严格递增，且全部 ≥ 0
4. 除末档外每档 `width` 必须为 `Some(w)` 且 `w > 0`；末档 `width` 必须为 `None`
5. `reject` 必须在合同界外侧：Upper 向 `reject ≥ max`，Lower 向 `reject ≤ min`
6. `PurchaseClause.direction` 不接受 `Range`
7. `Enforcement::Priced` 的 Spec 其 `direction` 不接受 `Range`——`Penalty` 只有一组 `tiers`
   与一个 `reject`，无法表达上下界各自的档位。挥发在本合同中为 Upper 向；区间型指标本期一律保持 `Hard`

第 3、5 条是安全性的核心：递减 rate 会让扣款被低估，reject 落在合同界内侧会让计价区间为空。

## C. LP 构造（`blend_kit_rs/src/optimizer.rs`）

### LpProblem 结构（`optimizer.rs:767`）

```rust
struct LpProblem {
    /// 参与 Σx=1 的前若干列(配比变量).
    ratio_count: usize,
    /// 总列数 = ratio_count + 档位变量数.
    n: usize,
    c: Vec<f64>,
    a_ub: Vec<Vec<f64>>,
    b_ub: Vec<f64>,
}
```

`solve()` 内需改三处，其余（`NonnegativeConeT(inequality_count + n)` 已覆盖全列，
档位变量自动非负；CSC 装配；可行性复核）保持不变：

1. 第 0 行（`ZeroConeT(1)` 对应的 Σx=1）只在 `0..ratio_count` 填 1.0
2. `raw_sum` 只对 `solution[..ratio_count]` 求和
3. 归一化只对 `solution[..ratio_count]` 做

**第 2、3 条是静默错误风险点**：现状是 `solution.iter().sum()` 扫全部列，
加入档位变量后会把扣款额算进配比和，再按错误分母归一化——不会 panic，只会给出错配方。

### 每个 Priced spec 生成的行

Upper 向（如灰 ≤ L，档位 `[(Some(w1), r1), (None, r2)]`，拒收线 R）：

```
a·x − d_1 − d_2 ≤ L          吸收行
d_1             ≤ w_1        有限档宽度行(每个有限档一行)
a·x             ≤ R          悬崖行
目标系数: c[d_1] = r_1, c[d_2] = r_2
```

Lower 向（如粘结 ≥ L，拒收线 R）：整行取负

```
−a·x − d_1 − d_2 ≤ −L
d_1              ≤ w_1
−a·x             ≤ −R
```

`a` 取自既有的 `MetricFormula::upper_constraint` / `lower_constraint`（`quality.rs:57/70`），
不新增指标线性化逻辑。

**不需要写"先填满 d_1 才能用 d_2"的次序约束**——`r_2 > r_1`，最小化目标时求解器自行按序填档。
这正是 rate 必须递增的原因。

### 买入侧价格修正

在 `solve_once`（`optimizer.rs:435`）构造 `costs`（`optimizer.rs:446`）之前完成：

1. 对每种煤，逐条 `PurchaseClause` 用该煤自身 `props` 的化验值算偏离量与扣款额
2. 越过任一 clause 的 `penalty.reject` → **剔出煤池并加 warning**（这种煤实际上不会被收货）
3. `cif_eff_i = fob_i·(1−M_eff_i)/(1−M合_i) + frt_i − Σ 扣款`
4. `contract_moisture` 为 None 时跳过水分折算项

## D. 输出（`model.rs`）

既有字段语义**一律不变**，只增不改：

```rust
pub struct IndicatorCheck {
    // ... 既有字段 ...
    /// 本项卖出侧扣款, 元/吨. 非计价项为 None.
    pub penalty_per_ton: Option<f64>,
}

pub struct CostBreakdown {
    // ... fob/frt/cif 保持报价原值 ...
    /// 买入侧扣款折扣 + 水分折算带来的到厂价修正合计, 元/吨. 负值 = 成本下降.
    pub purchase_adjust_per_ton: f64,
    /// 卖出侧扣款合计, 元/吨.
    pub penalty_per_ton: f64,
    /// 真实吨成本 = cif + purchase_adjust + penalty.
    pub net_per_ton: f64,
    pub total_purchase_adjust: Option<f64>,
    pub total_penalty: Option<f64>,
    pub total_net: Option<f64>,
}

pub struct OrderItem {
    // ... 既有字段 ...
    /// 该煤买入侧修正后的单价, 采购按此价核对.
    pub cif_eff: f64,
}
```

**解出的档位变量值直接就是每项扣款额**，不需另行计算——
`penalty_per_ton[k] = Σ_tier rate · d`，按 spec 分组即可。

## E. 前端（`doudou_blend/`）

1. **`src/types.ts`** 镜像 `PenaltyTier` / `Penalty` / `PurchaseClause` / `PurchaseTerms` /
   `Enforcement::Priced` / 新增输出字段。`npm run check:consistency` 会比对共享字段与枚举值。
2. **合同屏 `ContractScreen`**：每条 spec 加"计价"开关；开启后录档位表与拒收线。
   录入按合同原文两栏——"每 0.1%" + "扣 8 元"——前端换算成 `rate = 80.0` 存，
   用户不做心算，避免一个量级的录入错误。
3. **煤池屏 `CoalPoolScreen`**：一张全局扣款模板（默认套用到所有煤）+ 单煤保证值 +
   单煤覆盖开关。模板与覆盖合并后组装成 `PurchaseTerms` 传给 core。
   存储走 `storage.ts` / `cloudStorage.ts`，**不进 `coal_master.json`**。
4. **今日屏 `TodayScreen`**：成本卡从一行"到厂价"改为"到厂价 / 买入修正 / 预计扣款 / 净成本"；
   指标体检中超界的计价项显示"粘结 −1.8 点，扣 9 元/吨"，不再显示红色 FAIL。

## F. 测试

`blend_kit_rs`（fixture，不依赖 master 数据内容）：

1. **存量兼容**：不带 `penalty` / `purchase_terms` 的旧请求 JSON，求解结果与改动前逐字段一致
2. **凸性自动填档**：两档条款下偏离量跨档，断言一档填满、二档承接余量、扣款额等于手算值
3. **悬崖**：拒收线内可行；越过拒收线不可行且 `ok == false`
4. **经济正确性**：煤 A（2280 元，灰 10.0%）vs 煤 B（2200 元，灰 10.8%），
   灰分 80 元/吨·% 计价，断言 LP 选 B（净 2264 < 2280）
5. **符号方向**：买入侧扣款使 `net_per_ton` 下降；卖出侧扣款使其上升
6. **水分折算**：`fob` 按 `(1−M_eff)/(1−M合)` 折算而 `frt` 不折；超阈值双倍生效
7. **校验**：rate 递减 / reject 在合同界内侧 / Priced 缺 penalty / Range 买入 clause /
   Range 计价 spec，五种非法输入均返回 `Err`
8. **买入侧剔除**：单煤自身化验值越过其采购合同 reject 时被剔出煤池并产生 warning

`doudou_blend`：`npm test` 覆盖档位表录入的单位换算（"每 0.1% 扣 8 元" → 80.0）。

## G. 实施步骤

1. **核心**：`model.rs` 类型 + `quality.rs` 校验 + `optimizer.rs` 档位变量与目标函数 +
   买入侧价格修正 + 上述 Rust 测试。验收：`cargo test --release`、`cargo clippy --release -- -D warnings`
2. **前端**：`types.ts` 镜像 + 合同屏 / 煤池屏 / 今日屏。验收：`npm test`、`npm run build`
3. **验证**：`npm run check:consistency`、`npm run check:data`、CI 全绿

## 不在范围内

- **逾期未到货 30 元/吨**：与配比无关的常数，不影响最优解
- **质量不合格 25% 违约金**：该悬崖已由 `Penalty.reject` 的硬约束表达，不单独建模
- **结算台阶量化**：合同按 0.1% 台阶跳扣，台阶函数非凸、不可进 LP。
  本期 LP 与展示层统一用连续斜率，展示的"预计扣款"与实际结算相差不超过半个台阶。
  未来若要做，量化只在展示层进行，可复用既有的 `AcceptanceRule`（Truncate / Round / decimals）。
- **CSR 回归接入**：计价 CSR 仍用现有线性代理或已门控的评估器，不因本设计改变门控策略
