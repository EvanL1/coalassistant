# 合同扣款条款进入配煤最优化模型 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把合同的质量扣款条款建模为凸分段线性罚函数，使最优解从"最便宜的合规配方"变成"净成本最低的配方"。

**Architecture:** 卖出侧在 Clarabel LP 中为每个计价指标增开档位变量，罚款进目标函数，拒收线作硬约束；买入侧按单煤自身化验值把扣款与水分折算预处理进到厂价。两侧共用 `Penalty` / `PenaltyTier` 类型。`Enforcement` 新增 `Priced` 变体，默认仍是 `Hard`，存量请求行为不变。

**Tech Stack:** Rust (Clarabel LP, serde)、React 19 + TypeScript + Vite、vitest

**设计文档:** `docs/superpowers/specs/2026-09-18-contract-penalty-design.md`

---

## 执行前必读：两条会咬人的约束

1. **Rust 结构体字段与 `types.ts` 必须同commit修改。**
   `scripts/check_repository_consistency.mjs:122-137` 逐字段比对 `Coal` / `Spec` / `CostBreakdown` /
   `OrderItem` / `IndicatorCheck`，并比对 `Enforcement` 枚举变体。只改 Rust 不改 TS ⇒ CI 红。
   因此 Task 1 同时改两边。

2. **不要新增 `UserStorageSnapshot` 顶层键。**
   `blend_kit_server/src/database.rs:180` 显示用户存储是 PostgreSQL 显式列
   （`coal_prefs, contract, quantity, user_coals`），新增顶层键需要建表迁移 + 服务端改动。
   本计划改为：卖出侧条款存在 `Spec.penalty` 内（随 `contract` 列同步，零改动）；
   买入侧单煤条款存在 `CoalPref` 内（随 `coal_prefs` 列同步，零改动）；
   全局扣款模板只存 localStorage（`doudou_blend.penalty_template.v1`），**不跨设备同步**。
   跨设备同步模板需要另开一列 + 迁移，不在本计划内。

---

## 文件结构

| 文件 | 动作 | 职责 |
|---|---|---|
| `blend_kit_rs/src/model.rs` | 修改 | 新增 `PenaltyTier` / `Penalty` / `PurchaseClause` / `PurchaseTerms`；`Enforcement::Priced`；`Spec.penalty`；`Coal.purchase_terms`；输出结构体新字段 |
| `blend_kit_rs/src/quality.rs` | 修改 | `validate_request` 增加 7 条计价条款校验 |
| `blend_kit_rs/src/penalty.rs` | **新建** | 买入侧扣款折算：`tiered_amount` / `cif_eff` / `cif_eff_or_quoted` / `clauses_missing_assay`。与 LP 无关的纯算术，独立成文件便于单测 |
| `blend_kit_rs/src/optimizer.rs` | 修改 | `LpProblem` 支持档位列；计价约束行生成；罚款额回读；买入侧接线 |
| `blend_kit_rs/src/lib.rs` | 修改 | `mod penalty;`；`coal_from_tuple` 补 `purchase_terms: None`；端到端测试 |
| `doudou_blend/src/types.ts` | 修改 | 镜像上述 Rust 类型 |
| `doudou_blend/src/penalty.ts` | **新建** | 单位换算 `tierRate`、模板与单煤覆盖合并 `mergePurchaseTerms` |
| `doudou_blend/src/penaltyStorage.ts` | **新建** | 全局模板的 localStorage 读写 |
| `doudou_blend/src/storage.ts` | 修改 | `CoalPref` 增加买入侧保证值与覆盖字段 |
| `doudou_blend/src/screens/CostCard.tsx` | **新建** | 从 `TodayScreen.tsx`(916 行，已超 800 行目标)抽出成本卡，承载四行成本展示 |
| `doudou_blend/src/screens/ContractScreen.tsx` | 修改 | 每条 spec 的"计价"开关 + 档位表 + 拒收线录入 |
| `doudou_blend/src/screens/CoalPoolScreen.tsx` | 修改 | 全局模板入口 + 单煤保证值录入 |
| `doudou_blend/src/screens/TodayScreen.tsx` | 修改 | 改用 `CostCard`；指标体检显示扣款额 |

---

## Phase 1 — 核心算法（Task 1~5）

### Task 1: 数据模型与 TS 镜像（类型落地，行为不变）

**Files:**
- Modify: `blend_kit_rs/src/model.rs`
- Modify: `blend_kit_rs/src/optimizer.rs`（填中性值让编译通过）
- Modify: `blend_kit_rs/src/lib.rs`（`coal_from_tuple`）
- Modify: `doudou_blend/src/types.ts`
- Test: `blend_kit_rs/src/lib.rs`（tests 模块内）

- [ ] **Step 1: 写失败测试 —— 存量请求行为不变且新字段为中性值**

加到 `blend_kit_rs/src/lib.rs` 的 `mod tests` 内：

```rust
    /// 不含 penalty / purchase_terms 的旧请求: 求解结果不变, 新字段取中性值.
    #[test]
    fn test_legacy_request_keeps_neutral_penalty_fields() {
        let coals = vec![
            coal_from_tuple("甲", (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 50.0)),
            coal_from_tuple("乙", (0.8, 8.0, 26.0, 90.0, 18.0, 0.10, 66.0, 8.0, 1100.0, 50.0)),
        ];
        let request = BlendRequest {
            coals,
            specs: vec![Spec::upper("A", 10.0)],
            total_quantity: Some(1000.0),
            truncate_decimal: false,
        };
        let result = solve(&request);
        assert!(result.ok, "存量请求应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert_eq!(cost.penalty_per_ton, 0.0, "无计价条款时卖出侧扣款应为 0");
        assert_eq!(cost.purchase_adjust_per_ton, 0.0, "无采购条款时买入侧修正应为 0");
        assert!((cost.net_per_ton - cost.cif_per_ton).abs() < 1e-9, "净成本应等于到厂价");
        assert_eq!(cost.total_penalty, Some(0.0));

        for order in &result.orders {
            assert!(order.cif_eff_per_ton.is_finite(), "cif_eff_per_ton 应已填充");
        }
        for check in &result.indicator_check {
            assert_eq!(check.penalty_per_ton, None, "非计价指标扣款应为 None");
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cd blend_kit_rs && cargo test --release test_legacy_request_keeps_neutral_penalty_fields
```
预期：编译失败，`no field penalty_per_ton on type CostBreakdown` 等。

- [ ] **Step 3: 在 `model.rs` 新增类型**

在 `Direction` 枚举之后插入：

```rust
/// 扣款档位: 从上一档终点起, 覆盖 width 宽度的偏离, 按 rate 计费.
///
/// 合同写"每超 0.1% 扣 8 元/吨"时, rate = 80.0 (元/吨 per 1 个指标单位).
/// 单位换算由前端完成, core 只收换算后的斜率.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PenaltyTier {
    /// 本档覆盖的偏离宽度 (指标单位). None = 末档, 无上限.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// 元/吨 per 1 个指标单位.
    pub rate: f64,
}

/// 单项指标的计价条款.
///
/// tiers 的 rate 必须严格递增: 凸性是"档位变量可用 LP 精确表达"的前提,
/// 递减的 rate 会让求解器填错档并低估扣款.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Penalty {
    pub tiers: Vec<PenaltyTier>,
    /// 拒收线: 越过即硬不可行. Upper 向是上限, Lower 向是下限.
    pub reject: f64,
}

/// 单条采购合同计价条款 (买入侧).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PurchaseClause {
    pub indicator: String,
    /// 只接受 Upper (超标扣) / Lower (低于扣).
    pub direction: Direction,
    /// 该煤采购合同的保证值.
    pub guarantee: f64,
    pub penalty: Penalty,
}

/// 采购合同条款. 前端已把全局模板与单煤覆盖合并完毕后传入.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PurchaseTerms {
    pub clauses: Vec<PurchaseClause>,
    /// 合同水分 (%), 用于结算量折算. None = 不折算.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_moisture: Option<f64>,
    /// 超过该水分 (%) 时, 超出部分按 2 倍计入有效水分 M_eff. None = 不启用.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moisture_excess_double_threshold: Option<f64>,
}
```

- [ ] **Step 4: 在 `model.rs` 扩展既有结构体**

`Enforcement` 增加变体（放在 `Advisory` 之后，保持 `Hard` 为 `#[default]`）：

```rust
    /// 进入 LP, 但超界不判不可行, 而是按 Penalty 折算成元/吨计入目标;
    /// 越过 Penalty.reject 仍然硬不可行.
    Priced,
```

`Spec` 末尾增加字段：

```rust
    /// enforcement == Priced 时必填的计价条款.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub penalty: Option<Penalty>,
```

`Spec::upper` / `Spec::lower` / `Spec::range` 三个构造函数各补一行 `penalty: None,`。

`Coal` 末尾增加字段：

```rust
    /// 该煤采购合同条款. None = 按报价原值, 不做买入侧修正.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase_terms: Option<PurchaseTerms>,
```

`CostBreakdown` 末尾增加字段：

```rust
    /// 买入侧扣款折扣 + 水分折算带来的到厂价修正合计, 元/吨. 负值 = 成本下降.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
    #[serde(default)]
    pub purchase_adjust_per_ton: f64,
    /// 卖出侧质量扣款合计, 元/吨.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
    #[serde(default)]
    pub penalty_per_ton: f64,
    /// 真实吨成本 = cif + purchase_adjust + penalty.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 BlendResult 记录 (无此字段).
    #[serde(default)]
    pub net_per_ton: f64,
    pub total_purchase_adjust: Option<f64>,
    pub total_penalty: Option<f64>,
    pub total_net: Option<f64>,
```

`OrderItem` 末尾增加字段：

```rust
    /// 该煤买入侧修正后的单价 (元/吨), 采购按此价核对.
    /// `#[serde(default)]`: 兼容扣款条款上线前存量 OrderItem 记录 (无此字段).
    #[serde(default)]
    pub cif_eff_per_ton: f64,
```

`IndicatorCheck` 末尾增加字段：

```rust
    /// 本项卖出侧扣款, 元/吨. 非计价指标为 None.
    pub penalty_per_ton: Option<f64>,
```

- [ ] **Step 5: 在 `optimizer.rs` 填中性值让编译通过**

`solve_once` 内 `CostBreakdown` 构造处补：

```rust
        purchase_adjust_per_ton: 0.0,
        penalty_per_ton: 0.0,
        net_per_ton: cif_per_ton,
        total_purchase_adjust: request.total_quantity.map(|_| 0.0),
        total_penalty: request.total_quantity.map(|_| 0.0),
        total_net: request.total_quantity.map(|quantity| quantity * cif_per_ton),
```

`OrderItem` 构造处补 `cif_eff_per_ton: coal.cif(),`。

`IndicatorCheck` 的**两处**构造（`formulas.get` 失败分支与主分支）各补 `penalty_per_ton: None,`。

- [ ] **Step 6: 在 `lib.rs` 的 `coal_from_tuple` 补字段**

`Coal { ... }` 字面量增加 `purchase_terms: None,`。

- [ ] **Step 7: 编译，修掉所有 Spec/Coal 字面量构造点**

```bash
cd blend_kit_rs && cargo check --release 2>&1 | grep -E "^error|missing field" | head -20
```
逐个为报错处补 `penalty: None,` / `purchase_terms: None,`（`seed.rs` 可能也有构造点）。重复直到无错。

- [ ] **Step 8: 镜像到 `doudou_blend/src/types.ts`**

第 5 行改为：
```ts
export type Enforcement = "Hard" | "Soft" | "Advisory" | "Priced";
```

在 `Spec` 接口前插入：

```ts
/** 扣款档位: 覆盖 width 宽度的偏离, 按 rate (元/吨·指标单位) 计费. */
export interface PenaltyTier {
  /** 本档覆盖的偏离宽度; 省略 = 末档, 无上限. */
  width?: number | null;
  /** 元/吨 per 1 个指标单位. 合同"每 0.1% 扣 8 元" → 80. */
  rate: number;
}

/** 单项指标的计价条款. tiers 的 rate 必须严格递增(凸性). */
export interface Penalty {
  tiers: PenaltyTier[];
  /** 拒收线: 越过即硬不可行. */
  reject: number;
}

/** 单条采购合同计价条款 (买入侧). */
export interface PurchaseClause {
  indicator: string;
  /** 只接受 Upper / Lower. */
  direction: Direction;
  guarantee: number;
  penalty: Penalty;
}

/** 采购合同条款; 全局模板与单煤覆盖已在前端合并完毕. */
export interface PurchaseTerms {
  clauses: PurchaseClause[];
  /** 合同水分 (%). */
  contract_moisture?: number | null;
  /** 超过该水分 (%) 时, 超出部分按 2 倍计入有效水分 M_eff. */
  moisture_excess_double_threshold?: number | null;
}
```

`Spec` 接口末尾加 `penalty?: Penalty | null;`
`Coal` 接口末尾加 `purchase_terms?: PurchaseTerms | null;`
`CostBreakdown` 末尾加 (四个字段都标 `?`: 兼容扣款条款上线前存量 `BlendResult` 记录反序列化后
物理缺这些键; `backend.ts` 把存储 blob 直接断言成 `BlendResult`, 不标 `?` 会让 History 读取端
对旧记录类型检查通过却渲染出虚假的 0)：
```ts
  purchase_adjust_per_ton?: number;
  penalty_per_ton?: number;
  net_per_ton?: number;
  total_purchase_adjust?: number | null;
  total_penalty?: number | null;
  total_net?: number | null;
```
`OrderItem` 末尾加 `cif_eff_per_ton?: number;` 与 `cif_eff_amount?: number | null;` (同理标 `?`)
`IndicatorCheck` 末尾加 `penalty_per_ton?: number | null;`

`scripts/check_repository_consistency.mjs` 的 `sharedStructs` (约 122-137 行) 补四条, 否则
新增的四个类型 Rust↔TS 字段漂移不受 CI 保护：
```js
  [rustModelSource, "PenaltyTier", "PenaltyTier"],
  [rustModelSource, "Penalty", "Penalty"],
  [rustModelSource, "PurchaseClause", "PurchaseClause"],
  [rustModelSource, "PurchaseTerms", "PurchaseTerms"],
```

- [ ] **Step 9: 运行测试与一致性检查**

```bash
cd blend_kit_rs && cargo test --release test_legacy_request_keeps_neutral_penalty_fields
```
预期：PASS

```bash
cd blend_kit_rs && cargo test --release && cargo clippy --release -- -D warnings
cd ../doudou_blend && npm run check:consistency
```
预期：全部通过，既有测试无回归。

- [ ] **Step 10: 提交**

```bash
git add blend_kit_rs/src/model.rs blend_kit_rs/src/optimizer.rs blend_kit_rs/src/lib.rs doudou_blend/src/types.ts
git commit -m "feat(model): 新增扣款条款类型与 Enforcement::Priced, 行为暂不变"
```

---

### Task 2: 计价条款校验

**Files:**
- Modify: `blend_kit_rs/src/quality.rs`
- Test: `blend_kit_rs/src/quality.rs`（`mod tests` 内）

- [ ] **Step 1: 写失败测试 —— 五种非法输入**

加到 `blend_kit_rs/src/quality.rs` 的 `mod tests` 内：

```rust
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
                petrography: None,
                purchase_terms: None,
            }],
            specs: vec![spec],
            total_quantity: None,
            truncate_decimal: false,
        }
    }

    #[test]
    fn test_priced_spec_requires_penalty() {
        let mut spec = priced_spec(vec![PenaltyTier { width: None, rate: 80.0 }], 12.0);
        spec.penalty = None;
        assert!(validate_request(&request_with(spec)).is_err(), "Priced 缺 penalty 应报错");
    }

    #[test]
    fn test_penalty_rates_must_increase() {
        // 递减 rate 破坏凸性: LP 会填错档并低估扣款
        let spec = priced_spec(
            vec![
                PenaltyTier { width: Some(0.5), rate: 80.0 },
                PenaltyTier { width: None, rate: 40.0 },
            ],
            12.0,
        );
        assert!(validate_request(&request_with(spec)).is_err(), "rate 递减应报错");
    }

    #[test]
    fn test_penalty_reject_must_be_outside_contract_bound() {
        // Upper 向 reject 落在合同界内侧 ⇒ 计价区间为空
        let spec = priced_spec(vec![PenaltyTier { width: None, rate: 80.0 }], 9.0);
        assert!(validate_request(&request_with(spec)).is_err(), "reject 在合同界内侧应报错");
    }

    #[test]
    fn test_penalty_tail_tier_must_be_unbounded() {
        let spec = priced_spec(vec![PenaltyTier { width: Some(1.0), rate: 80.0 }], 12.0);
        assert!(validate_request(&request_with(spec)).is_err(), "末档必须无上限");
    }

    #[test]
    fn test_priced_spec_rejects_range_direction() {
        let mut spec = priced_spec(vec![PenaltyTier { width: None, rate: 80.0 }], 12.0);
        spec.direction = Direction::Range;
        spec.min = Some(5.0);
        assert!(validate_request(&request_with(spec)).is_err(), "Range 计价 spec 应报错");
    }

    #[test]
    fn test_valid_priced_spec_passes() {
        let spec = priced_spec(
            vec![
                PenaltyTier { width: Some(0.5), rate: 10.0 },
                PenaltyTier { width: None, rate: 20.0 },
            ],
            12.0,
        );
        assert!(validate_request(&request_with(spec)).is_ok(), "合法计价条款应通过");
    }
```

- [ ] **Step 2: 运行确认失败**

```bash
cd blend_kit_rs && cargo test --release test_penalty_rates_must_increase
```
预期：FAIL（`assertion failed: validate_request(...).is_err()`，因为尚未校验）

- [ ] **Step 3: 实现校验**

在 `quality.rs` 的 `validate_request` 里，spec 循环体末尾（`acceptance` 校验之后、`}` 之前）插入：

```rust
        if spec.enforcement == Enforcement::Priced {
            if spec.direction == Direction::Range {
                return Err(format!(
                    "{} 区间型指标不支持计价",
                    label_zh(&spec.indicator)
                ));
            }
            let Some(penalty) = &spec.penalty else {
                return Err(format!("{} 启用计价但缺扣款条款", label_zh(&spec.indicator)));
            };
            // Range 已在上面提前返回, 这里只剩 Upper/Lower; needs_max/needs_min 检查
            // (上文) 已保证对应边界存在, Some(..) else 只是防御式兜底.
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
```

同一 coal 循环里，`props` 校验之后追加买入侧校验（调用下方 `validate_purchase_terms`）：

```rust
        if let Some(terms) = &coal.purchase_terms {
            validate_purchase_terms(&coal.name, terms)?;
        }
```

在 `validate_request` 之后新增两个函数 —— `validate_purchase_terms` 校验买入侧
`PurchaseClause`（**含同指标重复检测**：一个煤挂两条同指标条款会在 Task 5 的
`effective_cif` 里被各自扣款重复计算，低估该煤成本，LP 因此过量买入，属于与凸性同一等级的
"静默算错钱"风险），`validate_penalty` 是双侧共用的核心规则（凸性 + 拒收线方向）：

```rust
fn validate_purchase_terms(coal_name: &str, terms: &PurchaseTerms) -> Result<(), String> {
    let mut indicators = std::collections::HashSet::new();
    for clause in &terms.clauses {
        if !INDICATORS.contains(&clause.indicator.as_str()) {
            return Err(format!("{} 采购条款指标未知: {}", coal_name, clause.indicator));
        }
        if !indicators.insert(clause.indicator.as_str()) {
            return Err(format!("{} 采购条款指标重复: {}", coal_name, clause.indicator));
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
        validate_penalty(&clause.indicator, clause.direction, clause.guarantee, &clause.penalty)
            .map_err(|err| format!("{coal_name} {err}"))?;
    }
    if terms.contract_moisture.is_some_and(|m| !m.is_finite() || !(0.0..=100.0).contains(&m)) {
        return Err(format!("{coal_name} 合同水分必须在 0~100 之间"));
    }
    if terms
        .moisture_excess_double_threshold
        .is_some_and(|t| !t.is_finite() || !(0.0..=100.0).contains(&t))
    {
        return Err(format!("{coal_name} 水分双倍阈值必须在 0~100 之间"));
    }
    Ok(())
}

/// 校验计价条款. 凸性(rate 递增)与拒收线方向是安全性的核心:
/// 前者错会让 LP 低估扣款, 后者错会让计价区间为空.
///
/// `bound` 是调用方按方向解出的合同边界 (Upper 传 max, Lower 传 min);
/// 两处调用方都已在调用前把 Range 拒掉, 也都已保证 bound 存在 —— 所以这里不需要
/// `Option<f64>` 双参数, 一个 `f64` 就够, 三条 ok_or_else/Range 的错误字符串都是死代码.
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
            return Err(format!("{label} 第 {} 档扣款率不能为负 (rate={})", index + 1, tier.rate));
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
```

`validate_penalty` 不需要 `pub(crate)`：两处调用方都在同一文件内。

`Direction::Range` 分支保持显式 `return Err(...)`，不要用 `unreachable!()`：两个 crate 的
release profile 都设了 `panic = "abort"`（`blend_kit_rs/Cargo.toml` / `blend_kit_server/Cargo.toml`），
一个可达的 panic 会直接 abort 整个服务进程、`CatchPanicLayer` 也拦不住，所有 in-flight 请求一起死。
校验函数属于请求边界代码，永远不能有这种失败模式；换成显式 `match` 分支（而非 `_ =>` 通配）已经能
让编译器在 `Direction` 新增变体时报错，`unreachable!()` 在这里没有额外收益，只有下行风险。

- [ ] **Step 4: 运行测试**

```bash
cd blend_kit_rs && cargo test --release penalty && cargo test --release && cargo clippy --release --all-targets -- -D warnings && cargo fmt --check
```
预期：新增测试全 PASS（卖出侧 + 买入侧 + 边界情况，覆盖比最初 6 个用例更全），既有测试无回归。

- [ ] **Step 5: 提交**

```bash
git add blend_kit_rs/src/quality.rs
git commit -m "feat(quality): 计价条款校验, 凸性与拒收线方向作硬校验"
```

---

### Task 3: `LpProblem` 支持档位列（结构改造，行为不变）

**Files:**
- Modify: `blend_kit_rs/src/optimizer.rs:767-855`
- Test: `blend_kit_rs/src/optimizer.rs`（新建 `mod tests`）

- [ ] **Step 1: 写失败测试 —— 只对配比列归一化**

在 `optimizer.rs` 文件末尾新增：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 档位列不得参与 Σx=1, 也不得被配比归一化缩放.
    #[test]
    fn test_lp_problem_excludes_extra_columns_from_sum() {
        let problem = LpProblem {
            ratio_count: 2,
            n: 3,
            c: vec![10.0, 20.0, 5.0],
            a_ub: Vec::new(),
            b_ub: Vec::new(),
        };
        let (solution, objective) = problem.solve().expect("应可解");
        assert_eq!(solution.len(), 3, "解应含全部列");
        assert!(
            (solution[0] + solution[1] - 1.0).abs() < 1e-6,
            "前 ratio_count 列之和应为 1, 实得 {}",
            solution[0] + solution[1]
        );
        assert!(solution[0] > 0.999, "应全选更便宜的第 0 列");
        assert!(solution[2].abs() < 1e-6, "附加列成本为正, 最优应取 0");
        assert!((objective - 10.0).abs() < 1e-4, "目标值应为 10, 实得 {objective}");
    }
}
```

- [ ] **Step 2: 运行确认失败**

```bash
cd blend_kit_rs && cargo test --release test_lp_problem_excludes_extra_columns_from_sum
```
预期：编译失败，`struct LpProblem has no field named ratio_count`。

- [ ] **Step 3: 改 `LpProblem` 结构体**

```rust
struct LpProblem {
    /// 参与 Σx=1 的前若干列 (配比变量).
    ratio_count: usize,
    /// 总列数 = ratio_count + 档位变量数.
    n: usize,
    c: Vec<f64>,
    a_ub: Vec<Vec<f64>>,
    b_ub: Vec<f64>,
}
```

- [ ] **Step 4: 改 `solve()` 的三处**

第一处 —— 等式行只覆盖配比列：

```rust
        for column in 0..self.ratio_count {
            triplets.push((0, column, 1.0));
        }
```

第二处 —— `raw_sum` 只统计配比列：

```rust
        let raw_sum: f64 = solution[..self.ratio_count].iter().sum();
```

第三处 —— 归一化只作用于配比列：

```rust
        let normalized_sum: f64 = solution[..self.ratio_count].iter().sum();
        if !normalized_sum.is_finite() || normalized_sum <= 0.0 {
            return None;
        }
        for value in &mut solution[..self.ratio_count] {
            *value /= normalized_sum;
        }
```

> 其余不动：`NonnegativeConeT(inequality_count + n)` 已覆盖全部列，档位变量自动非负；
> `build_csc` 与可行性复核对短于 `n` 的行按零填充处理，故既有约束行无需补宽。

- [ ] **Step 5: 更新唯一的构造点**

`solve_once` 内：

```rust
    let problem = LpProblem {
        ratio_count: count,
        n: count,
        c: costs,
        a_ub: inequalities,
        b_ub: bounds,
    };
    let (solution, _) = problem.solve()?;
    let ratios = solution[..count].to_vec();
```

（原 `let (ratios, _) = problem.solve()?;` 替换为上面两行。）

- [ ] **Step 6: 运行测试**

```bash
cd blend_kit_rs && cargo test --release && cargo clippy --release -- -D warnings
```
预期：新测试 PASS，既有测试全部无回归（此步行为应完全不变）。

- [ ] **Step 7: 提交**

```bash
git add blend_kit_rs/src/optimizer.rs
git commit -m "refactor(optimizer): LpProblem 区分配比列与附加列, 为档位变量让路"
```

---

### Task 4: 卖出侧罚项进 LP

**Files:**
- Modify: `blend_kit_rs/src/optimizer.rs`
- Test: `blend_kit_rs/src/optimizer.rs`、`blend_kit_rs/src/lib.rs`

- [ ] **Step 1: 写失败测试 —— 单位分母不变式**

加到 `optimizer.rs` 的 `mod tests`：

```rust
    /// 计价约束行的线性性依赖 den·x = Σx = 1.
    /// 若将来引入非单位分母的 MetricFormula, 罚项行会静默失真 —— 用这条测试钉住.
    #[test]
    fn test_all_formulas_have_unit_denominators() {
        let coals = vec![
            crate::coal_from_tuple("甲", (1.0, 9.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 1000.0, 0.0)),
            crate::coal_from_tuple("乙", (0.8, 8.0, 26.0, 90.0, 18.0, 0.10, 66.0, 8.0, 1100.0, 0.0)),
        ];
        let refs: Vec<&Coal> = coals.iter().collect();
        let formulas = build_formulas(&refs, &EvaluatorSet::default());
        assert!(!formulas.is_empty(), "应至少构造出一个公式");
        for (indicator, formula) in &formulas {
            assert!(
                formula.denominators.iter().all(|value| (value - 1.0).abs() < 1e-12),
                "{indicator} 的分母非单位, 计价约束将失真"
            );
        }
    }
```

- [ ] **Step 2: 写失败测试 —— 凸性自动填档、悬崖、经济正确性**

加到 `blend_kit_rs/src/lib.rs` 的 `mod tests`：

```rust
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
                    PenaltyTier { width: Some(1.0), rate: 10.0 },
                    PenaltyTier { width: None, rate: 30.0 },
                ],
                13.0,
            )],
            total_quantity: Some(100.0),
            truncate_decimal: false,
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
            ash.penalty_per_ton.is_some_and(|value| (value - 25.0).abs() < 1e-3),
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
                vec![PenaltyTier { width: None, rate: 10.0 }],
                13.0,
            )],
            total_quantity: None,
            truncate_decimal: false,
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
                coal_from_tuple("干净", (1.0, 10.0, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 2280.0, 0.0)),
                coal_from_tuple("便宜", (1.0, 10.8, 24.0, 88.0, 16.0, 0.10, 65.0, 8.0, 2200.0, 0.0)),
            ],
            specs: vec![priced_upper_spec(
                "A",
                10.0,
                vec![PenaltyTier { width: None, rate: 80.0 }],
                11.5,
            )],
            total_quantity: None,
            truncate_decimal: false,
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
```

- [ ] **Step 3: 运行确认失败**

```bash
cd blend_kit_rs && cargo test --release test_priced_penalty_fills_cheap_tier_first
```
预期：FAIL —— `扣款应为 25 元/吨, 实得 0`（罚项尚未实现）。

- [ ] **Step 4: 在 `optimizer.rs` 实现计价约束**

在 `solve_once` 上方新增：

```rust
/// 一条计价约束在 LP 中占用的档位列区间.
struct PricedBlock {
    indicator: String,
    /// 档位变量的起始列下标.
    offset: usize,
    tier_count: usize,
}
```

在 `solve_once` 内，`append_hard_model_domains(...)?;` 之后、构造 `LpProblem` 之前插入：

```rust
    // 计价约束: 每档一个变量, 罚款进目标. rate 递增保证求解器自行按序填档.
    let priced: Vec<&Spec> = specs
        .iter()
        .filter(|spec| spec.enforcement == Enforcement::Priced)
        .copied()
        .collect();
    let mut blocks: Vec<PricedBlock> = Vec::new();
    let mut total_columns = count;
    for spec in &priced {
        let penalty = spec.penalty.as_ref()?;
        blocks.push(PricedBlock {
            indicator: spec.indicator.clone(),
            offset: total_columns,
            tier_count: penalty.tiers.len(),
        });
        total_columns += penalty.tiers.len();
        for tier in &penalty.tiers {
            costs.push(tier.rate);
        }
    }

    for (spec, block) in priced.iter().zip(&blocks) {
        let formula = formulas.get(&spec.indicator)?;
        let penalty = spec.penalty.as_ref()?;
        let rule = acceptance_rule(spec, request.truncate_decimal);
        let margin = spec.margin.unwrap_or(0.0);

        // 合同界是精确的合同数字, 不加 margin; margin 只收紧拒收线.
        let (limit, reject) = match spec.direction {
            Direction::Upper => (
                effective_upper(spec.max?, &rule),
                effective_upper(penalty.reject, &rule) - margin,
            ),
            Direction::Lower => (
                effective_lower(spec.min?, &rule),
                effective_lower(penalty.reject, &rule) + margin,
            ),
            Direction::Range => return None,
        };

        // 吸收行: value 超出 limit 的部分由档位变量承接.
        let (mut row, bound) = match spec.direction {
            Direction::Upper => formula.upper_constraint(limit),
            _ => formula.lower_constraint(limit),
        };
        row.resize(total_columns, 0.0);
        for index in 0..block.tier_count {
            row[block.offset + index] = -1.0;
        }
        inequalities.push(row);
        bounds.push(bound);

        // 档宽行: 末档无上限, 不生成.
        for (index, tier) in penalty.tiers.iter().enumerate() {
            if let Some(width) = tier.width {
                let mut row = vec![0.0; total_columns];
                row[block.offset + index] = 1.0;
                inequalities.push(row);
                bounds.push(width);
            }
        }

        // 悬崖行: 越过拒收线硬不可行.
        let (mut row, bound) = match spec.direction {
            Direction::Upper => formula.upper_constraint(reject),
            _ => formula.lower_constraint(reject),
        };
        row.resize(total_columns, 0.0);
        inequalities.push(row);
        bounds.push(bound);
    }
```

把 `let costs: Vec<f64> = ...` 改为 `let mut costs: Vec<f64> = ...`。

`LpProblem` 构造改为：

```rust
    let problem = LpProblem {
        ratio_count: count,
        n: total_columns,
        c: costs,
        a_ub: inequalities,
        b_ub: bounds,
    };
    let (solution, _) = problem.solve()?;
    let ratios = solution[..count].to_vec();

    let mut penalty_by_indicator: HashMap<String, f64> = HashMap::new();
    for (spec, block) in priced.iter().zip(&blocks) {
        let penalty = spec.penalty.as_ref()?;
        let amount: f64 = penalty
            .tiers
            .iter()
            .enumerate()
            .map(|(index, tier)| tier.rate * solution[block.offset + index].max(0.0))
            .sum();
        penalty_by_indicator.insert(block.indicator.clone(), amount);
    }
    let penalty_per_ton: f64 = penalty_by_indicator.values().sum();
```

- [ ] **Step 5: 接进成本与体检输出**

`CostBreakdown` 构造处，把 Task 1 的中性值替换为：

```rust
        purchase_adjust_per_ton: 0.0,
        penalty_per_ton,
        net_per_ton: cif_per_ton + penalty_per_ton,
        total_purchase_adjust: request.total_quantity.map(|_| 0.0),
        total_penalty: request.total_quantity.map(|quantity| quantity * penalty_per_ton),
        total_net: request
            .total_quantity
            .map(|quantity| quantity * (cif_per_ton + penalty_per_ton)),
```

指标体检主分支内，`let mut outcome = check_value(...);` 之后插入：

```rust
        // 计价指标超合同界是预期行为(已折算成扣款), 只要未越拒收线就不判 Fail.
        // 拒收线本身是 LP 硬约束, 但非线性评估器的复算值可能与 LP 代理不同, 故仍显式复核.
        if let Some(spec) = spec {
            if spec.enforcement == Enforcement::Priced
                && outcome.status == EvaluationStatus::Fail
            {
                let within_reject = spec.penalty.as_ref().is_some_and(|penalty| {
                    match spec.direction {
                        Direction::Upper => evaluated <= penalty.reject + SOLUTION_TOLERANCE,
                        Direction::Lower => evaluated + SOLUTION_TOLERANCE >= penalty.reject,
                        Direction::Range => false,
                    }
                });
                if within_reject {
                    outcome.status = EvaluationStatus::TolerancePass;
                }
            }
        }
```

同一处 `IndicatorCheck` 构造的 `penalty_per_ton: None,` 改为：

```rust
            penalty_per_ton: penalty_by_indicator.get(indicator).copied(),
```

> `solve_once` 末尾"Hard spec 判 Fail 即返回 None"的早退逻辑按 `== Enforcement::Hard` 过滤，
> 新增的 `Priced` 自动被排除，无需改动。

- [ ] **Step 4b: `Priced` 必须与 `Hard` 一同参与选煤（本计划初稿遗漏）**

Task 4 之前 `Priced` 不产生任何 LP 行；实现后它产生拒收线硬行，于是"LP 里是硬约束、
选煤时却按非硬处理"成了 Task 4 自己引入的不一致。缺该指标的煤不被剔除，
`formula_for` 返回 `None`，整次求解退化为 `"约束冲突, LP 不可行"` 且 warning 为空——
同样煤池在 `Hard` 下本会剔煤、指名罪魁并正常求解。**可解的场景变成不可解。**

两处都改为 `matches!(spec.enforcement, Enforcement::Hard | Enforcement::Priced)`：
- `optimizer.rs` 的 `required` 集合构造（"只有 Hard 约束会剔除缺字段煤"）
- 其后的"缺少可用输入"预检查

紧随 `required` 的 CSR 特判 `if evaluators.csr.is_some() && required.remove("CSR")`
**无需改动**：它作用于 `required` 的内容而非 enforcement，`Priced` 的 CSR 指标进集合后
会被同一套 remove/extend 自动换成 S/A/V/G/Y/M 六项回归输入，语义正确。

配套测试 `test_priced_spec_culls_coal_missing_indicator`：煤池 = 缺该指标的煤 + 完整煤，
断言 `ok == true`、缺输入煤不在配方、warning 指名该煤。

- [ ] **Step 4c: 两条计价约束并存的测试（本计划初稿遗漏）**

只有一个块时 `offset` 恒等于 `count`，偏移算错**完全不可观测**——单块用例全绿。
必须有第二个块才真正检验块间偏移。`test_two_priced_specs_keep_separate_tier_blocks`
故意取不同档数（灰分 Upper 两档占 2 列 + 粘结 Lower 一档占 1 列），等宽会掩盖差一错误。
断言：两个指标各自的 `penalty_per_ton` 互不串块（25 / 10），合计 35，
且 `net_per_ton == cif_per_ton + 35`。

> 已用变异测试验证其有效性：把 `offset: total_columns` 改成 `offset: count`
> （单块时与正确实现等价）后，5 条单块用例**全部仍然通过**，只有这条两块用例失败。

- [ ] **Step 4d: `margin` 的两条不变式（本计划初稿遗漏）**

`margin` 只收紧拒收线、绝不移动合同界。合同界是结算用的精确合同数字，移动它会让
每吨凭空多扣 `margin × rate`，**不报错、只是安静地算贵**。初稿全部计价用例的
`margin` 都是 `None`，`margin == 0.0` 使该不变式在整个测试套件中不可观测——
代码写对了，却零保护。审查用变异测试证实：把 `margin` 同时加到 `limit` 上，
103 条测试**全绿**。

- `test_priced_margin_does_not_shift_contract_limit`：同一场景跑两遍
  （`margin: None` 与 `margin: Some(0.5)`），断言 `penalty_per_ton` 相等（容差 1e-6）。
  灰分取 11.0 而非 11.5，是为了让变异态仍可解，从而由相等断言抓住差异，
  而不是靠 `result.ok` 断言间接抓住。
- `test_priced_margin_tightens_reject_line`：灰分 11.25 落在 [拒收线 − margin, 拒收线]
  内，`None` 时可解、`Some(1.0)` 时不可行。缺了它，删光 `margin` 的作用即可伪造前一条通过。

> 变异验证：`margin` 误加到 `limit` → 仅第一条失败（无 margin 10.0 / 有 margin 25.0）；
> 抽掉 `margin` 对 `reject` 的作用 → 仅第二条失败。两次都是 94 过 1 failed。

- [x] **✅ 已修：计价解约 27% 被误判不可行（Task 4 期间发现并修复）**

`LpProblem::solve` 末尾的可行性复算用**绝对**容差 `SOLUTION_TOLERANCE = 1e-8`：

```rust
row.iter().zip(&solution).map(...).sum::<f64>() <= bound + SOLUTION_TOLERANCE
```

配比列在复算前被归一化重投影，残差收缩到 ~1e-13，所以硬约束一直没被咬到。
**档位列没有任何归一化**，残差就是 Clarabel 内点法的收敛余量 ~1e-8..1e-7，
而吸收行在最优解处**必然取等**（目标函数把 Σd 压到恰好等于偏离量），
于是复算把正常解判成越界 → `solve_once` 返回 `None` → `"约束冲突, LP 不可行"`。

实测（合同上限 10.0，单档 rate 10，扫描 ash 10.0~13.0 × reject 11.0~15.0）：
**92 组良态输入中 25 组（27%）被误判不可行**，且分布无规律（ash=12.75 通过而
12.50 失败），纯粹是条件数运气。诊断样本：ash=12.5 / reject=13.0 解出
`x=[0.999999999999865, 2.499999961849723]`，吸收行残差 **3.815e-8 > 1e-8**。

这与已关闭的"档位列未随配比列归一化"不是同一回事：那条的量级被 `SOLUTION_TOLERANCE`
兜住，这条的残差是容差的 4 倍且可复现证伪。

**修复：相对可行性判据。** 新增独立常量（不复用、不放宽 `SOLUTION_TOLERANCE`，
后者还守着 `raw_sum`、岩相比较与拒收线复核，必须保持紧）：

```rust
const FEASIBILITY_TOLERANCE: f64 = 1e-7;
...
activity <= bound + FEASIBILITY_TOLERANCE * (1.0 + magnitude)
```

`magnitude = Σ|aᵢxᵢ|` 再与 `|bound|` 取大，容限随行与解的量级缩放。

**常量取值来自实测，非拍脑袋。** 92 组输入采样 156 行，绝对 1e-8 下 22 行被误判，
最坏残差 **3.815e-8**（对应 `magnitude = 5.0`），全样本最大
`residual/(1+magnitude) = 6.36e-9`。取 **1e-7**：最小的、对最坏实测值留出
一个数量级以上余量（约 16×）的整数量级常量。1e-8 虽也能跑满 92/92，但余量仅 1.57×，
不够。

**第二道门限同因同修。** 只改复算只到 89/92：余下 3 例（ash 恰好压合同上限、
偏离为 0）卡在**非负性门限**——最优解 d=0 时内点法从下方逼近，档位列返回
`-1.567e-8`，被 `*value >= -SOLUTION_TOLERANCE` 拦下。同一根因（档位列不归一化），
故同样放宽到 `FEASIBILITY_TOLERANCE`；其后立即 `max(0.0)` 夹回，且仍比
`OUTPUT_RATIO_TOLERANCE`(1e-5) 严两个数量级。`raw_sum` 保持 `SOLUTION_TOLERANCE`。
修复后 **92/92**。

**墙还是墙（实测）。** 拒收线 12.0：正好压线可行；超线 1e-9 / 1e-7 / 1e-5 / 0.01
全部拦截。理论放行量 `FEASIBILITY_TOLERANCE × (1+magnitude)` ≈ 1e-7~5e-7 指标单位，
比化验 0.01% 的分辨率细 4~5 个数量级；实测更紧（超线即转为 LP 真不可行，
由求解器状态门拦下，容限根本用不上）。

配套测试：`test_priced_solution_survives_feasibility_recheck`（ash 12.5/reject 13.0，
原失败点，须解出 25 元/吨）、`test_priced_zero_deviation_survives_nonnegativity_gate`
（零偏离，扣款须为 0）、`test_reject_line_still_blocks_one_assay_increment`
（压线可行 / 超线 0.01% 不可行）。

> 变异验证：复算退回绝对 `SOLUTION_TOLERANCE` → 前者与墙测试失败（96 过 2 failed）；
> 非负性门限退回 `SOLUTION_TOLERANCE` → 零偏离测试失败（97 过 1 failed）。

**容限必须两侧同改。** 体检的 `Fail → TolerancePass` 复核也在拿复算值比拒收线，
只改 LP 侧会让两者判据不一致：LP 放行 `FEASIBILITY_TOLERANCE*(1+magnitude)`，
体检却卡绝对 1e-8，于是 LP 认可的解被判 `Fail`，再经 `finalize_quality_status`
变成 `NeedsReview`——正确配方被盖上"需要复核"。实测业务量级内可复现
（便宜脏煤压悬崖，超线 1.0e-8~9.1e-8）。两臂统一为
`FEASIBILITY_TOLERANCE * (1.0 + penalty.reject.abs())`，由
`test_priced_indicator_check_tolerance_matches_lp_wall` 钉住（复算值
12.75000003279332539，超线 3.28e-8，旧容限兜不住）。

> 变异验证：该臂退回 `SOLUTION_TOLERANCE` → 仅此测试失败（98 过 1 failed，`left: Fail`）。

- [ ] **Step 6: 运行测试**

```bash
cd blend_kit_rs && cargo test --release && cargo clippy --release -- -D warnings
```
预期：Task 4 的 14 个新测试 PASS（optimizer.rs 1 条不变式 + lib.rs 13 条行为），既有测试无回归。

- [ ] **Step 7: 提交**

```bash
git add blend_kit_rs/src/optimizer.rs blend_kit_rs/src/lib.rs
git commit -m "feat(optimizer): 卖出侧质量扣款进 LP 目标函数, 拒收线作硬约束"
```

---

### Task 5: 买入侧到厂价修正

**Files:**
- Create: `blend_kit_rs/src/penalty.rs`
- Modify: `blend_kit_rs/src/lib.rs`（`mod penalty;`）
- Modify: `blend_kit_rs/src/optimizer.rs`
- Test: `blend_kit_rs/src/penalty.rs`、`blend_kit_rs/src/lib.rs`

- [ ] **Step 1: 写失败测试 —— 新建 `penalty.rs` 连同单测**

创建 `blend_kit_rs/src/penalty.rs`：

```rust
//! 买入侧扣款折算: 按单煤自身化验值与其采购合同条款修正到厂价.
//!
//! 与卖出侧的区别:
//!   - 买入侧扣款由该煤自身化验值决定, 是常数, 直接折进 LP 目标的成本系数;
//!   - 卖出侧扣款由混煤指标决定, 是决策变量的函数, 必须进 LP 作档位变量.
//!
//! 符号: 买入侧扣款让你**少付钱**, 因此是折扣(降成本), 与卖出侧方向相反.

use crate::model::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(width: Option<f64>, rate: f64) -> PenaltyTier {
        PenaltyTier { width, rate }
    }

    #[test]
    fn test_tiered_amount_fills_cheap_tier_first() {
        let tiers = vec![tier(Some(1.0), 10.0), tier(None, 30.0)];
        assert!((tiered_amount(&tiers, 0.0) - 0.0).abs() < 1e-9);
        assert!((tiered_amount(&tiers, 0.5) - 5.0).abs() < 1e-9);
        assert!((tiered_amount(&tiers, 1.0) - 10.0).abs() < 1e-9);
        // 1.5 = 一档 1.0×10 + 二档 0.5×30
        assert!((tiered_amount(&tiers, 1.5) - 25.0).abs() < 1e-9);
    }

    fn coal_with(props: &[(&str, f64)], terms: Option<PurchaseTerms>) -> Coal {
        let mut map = std::collections::HashMap::new();
        for (key, value) in props {
            map.insert((*key).to_string(), *value);
        }
        Coal {
            name: "甲".into(),
            props: map,
            fob: 1000.0,
            frt: 100.0,
            petrography: None,
            purchase_terms: terms,
        }
    }

    #[test]
    fn test_no_terms_keeps_quoted_price() {
        let coal = coal_with(&[("A", 10.0)], None);
        assert_eq!(effective_cif(&coal), Some(1100.0));
    }

    #[test]
    fn test_deduction_is_a_discount() {
        // 保证 10%, 实测 10.5%, 80 元/吨·% ⇒ 扣 40 元 ⇒ 成本下降
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty { tiers: vec![tier(None, 80.0)], reject: 12.0 },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        let coal = coal_with(&[("A", 10.5)], Some(terms));
        assert!((effective_cif(&coal).unwrap() - 1060.0).abs() < 1e-9);
    }

    #[test]
    fn test_within_guarantee_has_no_deduction() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty { tiers: vec![tier(None, 80.0)], reject: 12.0 },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        let coal = coal_with(&[("A", 9.2)], Some(terms));
        assert_eq!(effective_cif(&coal), Some(1100.0));
    }

    #[test]
    fn test_beyond_reject_returns_none() {
        let terms = PurchaseTerms {
            clauses: vec![PurchaseClause {
                indicator: "A".into(),
                direction: Direction::Upper,
                guarantee: 10.0,
                penalty: Penalty { tiers: vec![tier(None, 80.0)], reject: 12.0 },
            }],
            contract_moisture: None,
            moisture_excess_double_threshold: None,
        };
        let coal = coal_with(&[("A", 13.0)], Some(terms));
        assert_eq!(effective_cif(&coal), None, "越过采购合同拒收线的煤不应入池");
    }

    #[test]
    fn test_moisture_discounts_fob_but_not_freight() {
        // 结算量 = 净重×(1−10%)/(1−8%); 运费按实收湿重付, 不打折
        let terms = PurchaseTerms {
            clauses: Vec::new(),
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: None,
        };
        let coal = coal_with(&[("M", 10.0)], Some(terms));
        let expected = 1000.0 * (0.90 / 0.92) + 100.0;
        assert!(
            (effective_cif(&coal).unwrap() - expected).abs() < 1e-6,
            "应为 {expected}, 实得 {:?}",
            effective_cif(&coal)
        );
    }

    #[test]
    fn test_moisture_excess_double_threshold() {
        // 实测 14% 超阈值 12% ⇒ 有效水分 2×14−12 = 16%
        let terms = PurchaseTerms {
            clauses: Vec::new(),
            contract_moisture: Some(8.0),
            moisture_excess_double_threshold: Some(12.0),
        };
        let coal = coal_with(&[("M", 14.0)], Some(terms));
        let expected = 1000.0 * (0.84 / 0.92) + 100.0;
        assert!((effective_cif(&coal).unwrap() - expected).abs() < 1e-6);
    }
}
```

- [ ] **Step 2: 运行确认失败**

```bash
cd blend_kit_rs && cargo test --release tiered_amount
```
预期：编译失败（`penalty` 模块未挂载、函数不存在）。

- [ ] **Step 3: 实现 `penalty.rs` 的两个函数**

在 `penalty.rs` 的 `use crate::model::*;` 之后、`#[cfg(test)]` 之前插入：

```rust
/// 凸分段线性扣款: 偏离量按档位依次计费, 便宜档先填满.
pub(crate) fn tiered_amount(tiers: &[PenaltyTier], deviation: f64) -> f64 {
    let mut remaining = deviation.max(0.0);
    let mut amount = 0.0;
    for tier in tiers {
        if remaining <= 0.0 {
            break;
        }
        let used = tier.width.map_or(remaining, |width| remaining.min(width));
        amount += used * tier.rate;
        remaining -= used;
    }
    amount
}

/// 按采购合同条款折算后的到厂价.
///
/// 返回 None 表示该煤自身化验值已越过其采购合同拒收线 —— 这种货实际收不进来, 应剔出煤池.
pub(crate) fn effective_cif(coal: &Coal) -> Option<f64> {
    let Some(terms) = &coal.purchase_terms else {
        return Some(coal.cif());
    };

    let mut deduction = 0.0;
    for clause in &terms.clauses {
        let Some(value) = coal.get(&clause.indicator) else {
            continue;
        };
        let (deviation, beyond_reject) = match clause.direction {
            Direction::Upper => (value - clause.guarantee, value > clause.penalty.reject),
            Direction::Lower => (clause.guarantee - value, value < clause.penalty.reject),
            Direction::Range => continue,
        };
        if beyond_reject {
            return None;
        }
        if deviation > 0.0 {
            deduction += tiered_amount(&clause.penalty.tiers, deviation);
        }
    }

    // 结算量 = 实收净重 × (1−实际水分)/(1−合同水分); 运费按实收湿重付, 不参与折算.
    let moisture_factor = match (terms.contract_moisture, coal.get("M")) {
        (Some(contract), Some(actual)) if contract < 100.0 => {
            let effective = match terms.moisture_excess_double_threshold {
                Some(threshold) if actual > threshold => 2.0 * actual - threshold,
                _ => actual,
            };
            (1.0 - effective / 100.0) / (1.0 - contract / 100.0)
        }
        _ => 1.0,
    };

    Some(coal.fob * moisture_factor + coal.frt - deduction)
}
```

- [ ] **Step 4: 挂载模块**

`blend_kit_rs/src/lib.rs` 的模块声明区加一行：

```rust
mod penalty;
```

- [ ] **Step 5: 运行单测**

```bash
cd blend_kit_rs && cargo test --release --lib penalty::
```
预期：7 个测试全 PASS。

- [ ] **Step 6: 写失败测试 —— 接进求解器**

加到 `blend_kit_rs/src/lib.rs` 的 `mod tests`：

```rust
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
        };
        let result = solve(&request);
        assert!(result.ok, "应可解: {:?}", result.reason);

        let cost = result.cost.expect("应有成本");
        assert!((cost.cif_per_ton - 1100.0).abs() < 1e-6, "cif 应保持报价原值");
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
        assert_eq!(cost.total_net, Some(10600.0));

        let order = result.orders.first().expect("应有订单");
        assert!((order.cif_eff_per_ton - 1060.0).abs() < 1e-6, "订单单价应为修正后价");
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
        };
        let result = solve(&request);
        assert!(result.ok, "剩余煤应仍可解");
        assert!(!result.recipe.contains_key("脏煤"), "越拒收线的煤不应入配方");
        assert!(
            result.warnings.iter().any(|warning| warning.contains("脏煤")),
            "应有剔除警告, 实得 {:?}",
            result.warnings
        );
    }
```

- [ ] **Step 7: 运行确认失败**

```bash
cd blend_kit_rs && cargo test --release test_purchase_deduction_lowers_net_cost
```
预期：FAIL —— `买入修正应为 −40, 实得 0`。

- [ ] **Step 8: 在 `optimizer.rs` 接线**

文件顶部 `use` 区加：

```rust
use crate::penalty::effective_cif;
```

在 `solve_with_evaluators` 的候选煤筛选循环中（`if missing.is_empty() { kept.push(coal); }` 那段），
把 push 分支改为：

```rust
        if missing.is_empty() {
            if effective_cif(coal).is_none() {
                warnings.push(format!("剔除 {}: 化验值越过采购合同拒收线", coal.name));
            } else {
                kept.push(coal);
            }
        } else {
```

在 `solve_once` 内把成本系数改为修正后价（`let mut costs` 那一行）：

```rust
    // 买入侧扣款与水分折算已折进成本系数; 越拒收线的煤在候选筛选阶段已剔除, 此处兜底用报价.
    let mut costs: Vec<f64> = coals
        .iter()
        .map(|coal| effective_cif(coal).unwrap_or_else(|| coal.cif()))
        .collect();
```

在 `cif_per_ton` 计算之后插入买入修正额：

```rust
    let effective_cif_per_ton: f64 = coals
        .iter()
        .zip(&ratios)
        .map(|(coal, ratio)| effective_cif(coal).unwrap_or_else(|| coal.cif()) * ratio)
        .sum();
    let purchase_adjust_per_ton = effective_cif_per_ton - cif_per_ton;
```

`CostBreakdown` 构造改为：

```rust
        purchase_adjust_per_ton,
        penalty_per_ton,
        net_per_ton: cif_per_ton + purchase_adjust_per_ton + penalty_per_ton,
        total_purchase_adjust: request
            .total_quantity
            .map(|quantity| quantity * purchase_adjust_per_ton),
        total_penalty: request.total_quantity.map(|quantity| quantity * penalty_per_ton),
        total_net: request.total_quantity.map(|quantity| {
            quantity * (cif_per_ton + purchase_adjust_per_ton + penalty_per_ton)
        }),
```

`OrderItem` 构造的 `cif_eff_per_ton` 改为：

```rust
                cif_eff_per_ton: effective_cif(coal).unwrap_or_else(|| coal.cif()),
```

- [ ] **Step 9: 运行全部测试**

```bash
cd blend_kit_rs && cargo test --release && cargo clippy --release -- -D warnings && cargo fmt --check
```
预期：全部 PASS。

- [ ] **Step 10: 提交**

```bash
git add blend_kit_rs/src/penalty.rs blend_kit_rs/src/optimizer.rs blend_kit_rs/src/lib.rs
git commit -m "feat(penalty): 买入侧按采购合同扣款与水分折算修正到厂价"
```

- [ ] **Step 11: 实现修订 (质量复审后)**

**以下条目优先于上面 Step 1~9 的代码块** —— 那些是初稿, 实现时按复审意见改了八处.
Task 6~8 按本节的签名与字段写前端.

1. **函数改名**: `effective_cif` → `cif_eff`, 与它喂的字段 `cif_eff_per_ton` 对齐
   (改函数不改字段: 字段已序列化、已镜像进 TS, Task 1 已改过一次名).
   `tiered_amount` 降为私有 `fn` (仅文件内单一调用方).

2. **返回类型**: `cif_eff(coal) -> Result<f64, CullReason>`, 不再是 `Option`.
   ```rust
   pub(crate) struct CullReason { pub indicator: String, pub value: f64, pub reject: f64 }
   ```
   剔煤告警因此能指名道姓, 与同处的"缺指标"分支对齐:
   `剔除 脏煤: 灰 实测 13 越过采购合同拒收线 12`.
   另有 `cif_eff_or_quoted(coal) -> f64` 收敛三处 `unwrap_or_else(|_| coal.cif())` 兜底.

3. **水分两机制互斥** (`quality.rs::validate_purchase_terms`): `contract_moisture.is_some()`
   且存在 `indicator == "M"` 的条款时报错. 扣量(结算量折算)与扣价(元/吨)是真实合同里
   二选一的两种写法, 同时配置会让每吨水分被扣两遍, 且账面完全合理、不报任何错 ——
   全局模板一旦同时带上, 整个煤池被系统性低估.

4. **缺化验值的条款只告警不剔煤**: `clauses_missing_assay(coal) -> Vec<&str>`, 在候选筛选
   处发一次告警 (`甲: 采购条款缺化验值 硫, 本次不计买入扣款`). 全局模板把同一套条款套到
   每个煤上, 化验单缺项是常态, 报错会连带废掉大批本可正常采购的货.

5. **`OrderItem` 新增 `cif_eff_amount: Option<f64>`** (`#[serde(default)]`, 同步镜像进
   `types.ts`). 原先 `cif_amount` 是报价口径而 `cif_eff_per_ton` 是修正口径, 同一行里
   两种钱混着放, 前端取 `cif_amount` 当订单金额会静默吞掉买入折扣. 口径:
   `Σ cif_eff_amount + 卖出扣款 = total_net`; `cif_amount` 仅作报价展示.

6. **买入修正额逐煤算**, 而非两个千元级总额相减:
   ```rust
   let purchase_adjust_per_ton: f64 = coals.iter().zip(&ratios)
       .map(|(coal, ratio)| (cif_eff_or_quoted(coal) - coal.cif()) * ratio)
       .sum();
   ```
   相减形式 (`Σ(fob+frt)·x − (Σfob·x + Σfrt·x)`) 数学等价但浮点上不等: 实测 20 万组
   无条款配方里 44% 残留 ~1e-13, 乘总吨数后放大成一笔并不存在的买入修正.
   逐煤形式无条款时每项恰为 `0.0·x`, 求和仍是精确 0.

7. **`solve_json` 测试断言实际 JSON 键**, 不反序列化回 `BlendResult`:
   结构体往返会让 `#[serde(rename)]` 原样穿过, 测试照样绿而前端拿不到字段.
   用 `serde_json::Value` 取 `cost.net_per_ton` / `cost.purchase_adjust_per_ton` /
   `orders[0].cif_eff_per_ton` / `orders[0].cif_eff_amount` 四个键名并校值.

8. **`Direction::Range` 保持 `continue`, 不改 `unreachable!()`**: 两个 crate 的 release
   profile 都设了 `panic = "abort"`, 真被走到会直接崩掉整个 server 进程, `CatchPanicLayer`
   接不住. 上游守卫写在注释里 (`quality.rs` 的 `Direction::Range` 分支).

---

## Phase 2 — 前端（Task 6~8）

### Task 6: 前端扣款工具模块

**Files:**
- Create: `doudou_blend/src/penalty.ts`
- Create: `doudou_blend/src/penalty.test.ts`
- Create: `doudou_blend/src/penaltyStorage.ts`
- Modify: `doudou_blend/src/storage.ts`（`CoalPref` 增字段）

> **实现记录（两轮质量审查后修订）：** 下面的代码块是这个任务实际交付的形状,
> 已经吸收了质量审查发现的全部问题（首轮 4 个 BLOCKING + 二轮 3 个 Low）,
> 不是最初草稿。与最初设计的差异见文末小结。

- [ ] **Step 1: 写失败测试**

创建 `doudou_blend/src/penalty.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { mergePurchaseTerms, tierRate } from "./penalty";
import type { PenaltyTemplate } from "./penalty";

describe("tierRate", () => {
  it("把合同原文的每 0.1% 扣 8 元换算成 80 元/吨·%", () => {
    expect(tierRate({ step: 0.1, amount: 8 })).toBeCloseTo(80, 9);
  });

  it("每 0.01% 扣 2 元换算成 200 元/吨·%", () => {
    expect(tierRate({ step: 0.01, amount: 2 })).toBeCloseTo(200, 9);
  });

  it("每 1 点扣 5 元保持 5", () => {
    expect(tierRate({ step: 1, amount: 5 })).toBeCloseTo(5, 9);
  });

  it("0 元是合法的零费率档位, 不是错误信号", () => {
    expect(tierRate({ step: 1, amount: 0 })).toBe(0);
  });

  it("step 非法(非正数/非有限)返回 null 而不是 Infinity", () => {
    expect(tierRate({ step: 0, amount: 8 })).toBeNull();
    expect(tierRate({ step: -1, amount: 8 })).toBeNull();
    expect(tierRate({ step: Number.NaN, amount: 8 })).toBeNull();
  });

  it("amount 非法(负数/非有限)返回 null —— 负费率等于扣款倒贴钱", () => {
    expect(tierRate({ step: 0.1, amount: -8 })).toBeNull();
    expect(tierRate({ step: 0.1, amount: Number.NaN })).toBeNull();
  });
});

const template: PenaltyTemplate = {
  contract_moisture: 8,
  moisture_excess_double_threshold: 12,
  clauses: [
    {
      indicator: "A",
      direction: "Upper",
      penalty: { tiers: [{ rate: 80 }], reject: 12 },
    },
    {
      indicator: "G",
      direction: "Lower",
      penalty: { tiers: [{ rate: 5 }], reject: 80 },
    },
  ],
};

describe("mergePurchaseTerms", () => {
  it("没有保证值的指标不产出条款, 也不算孤儿", () => {
    const merged = mergePurchaseTerms(template, undefined, {});
    expect(merged.terms).toBeNull();
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("按保证值从模板生成条款, 水分字段一并带过来", () => {
    const merged = mergePurchaseTerms(template, undefined, { A: 10, G: 85 });
    expect(merged.terms?.clauses).toHaveLength(2);
    expect(merged.terms?.contract_moisture).toBe(8);
    expect(merged.terms?.moisture_excess_double_threshold).toBe(12);
    const ash = merged.terms?.clauses.find((clause) => clause.indicator === "A");
    expect(ash?.guarantee).toBe(10);
    expect(ash?.penalty.tiers[0].rate).toBe(80);
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("单煤覆盖优先于模板", () => {
    const merged = mergePurchaseTerms(
      template,
      { clauses: [{ indicator: "A", direction: "Upper", penalty: { tiers: [{ rate: 120 }], reject: 11 } }] },
      { A: 10, G: 85 },
    );
    const ash = merged.terms?.clauses.find((clause) => clause.indicator === "A");
    expect(ash?.penalty.tiers[0].rate).toBe(120);
    expect(ash?.penalty.reject).toBe(11);
    const cohesion = merged.terms?.clauses.find((clause) => clause.indicator === "G");
    expect(cohesion?.penalty.tiers[0].rate).toBe(5);
  });

  it("只给部分指标保证值时, 只产出对应条款, 未提供保证值的指标既不出条款也不算孤儿", () => {
    const merged = mergePurchaseTerms(template, undefined, { A: 10 });
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.terms?.clauses[0].indicator).toBe("A");
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("单煤覆盖可以新增模板没有的指标(覆盖专属条款)", () => {
    const merged = mergePurchaseTerms(
      template,
      { clauses: [{ indicator: "M", direction: "Upper", penalty: { tiers: [{ rate: 30 }], reject: 15 } }] },
      { M: 9 },
    );
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.terms?.clauses[0].indicator).toBe("M");
  });

  it("单煤覆盖可以排除模板条款: 即使有保证值也不产出, 且不计入孤儿", () => {
    const merged = mergePurchaseTerms(
      template,
      { excluded_indicators: ["G"] },
      { A: 10, G: 85 },
    );
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.terms?.clauses.some((clause) => clause.indicator === "G")).toBe(false);
    expect(merged.orphanedGuarantees).toEqual([]);
  });

  it("模板整体缺失(如跨设备未同步): 有保证值却找不到条款要报孤儿, 不能静默吃掉", () => {
    const merged = mergePurchaseTerms(null, undefined, { A: 10 });
    expect(merged.terms).toBeNull();
    expect(merged.orphanedGuarantees).toEqual(["A"]);
  });

  it("部分指标的模板条款缺失: 有条款的照常产出, 缺条款的报孤儿而不是被吞掉", () => {
    const merged = mergePurchaseTerms(
      null,
      { clauses: [{ indicator: "A", direction: "Upper", penalty: { tiers: [{ rate: 80 }], reject: 12 } }] },
      { A: 10, G: 85 },
    );
    expect(merged.terms?.clauses).toHaveLength(1);
    expect(merged.orphanedGuarantees).toEqual(["G"]);
  });

  it("同一指标既在覆盖条款里又被排除时, 排除生效(排除是比覆盖条款更具体的信号: 覆盖条款说明'这项按这个价算', 排除说明'这项压根不适用', 后者更接近用户的真实意图)", () => {
    const merged = mergePurchaseTerms(
      null,
      {
        clauses: [{ indicator: "A", direction: "Upper", penalty: { tiers: [{ rate: 120 }], reject: 11 } }],
        excluded_indicators: ["A"],
      },
      { A: 10 },
    );
    expect(merged.terms).toBeNull();
    expect(merged.orphanedGuarantees).toEqual([]);
  });
});

// contract_moisture / moisture_excess_double_threshold 共用同一条继承规则,
// 两个字段各测一遍三态 + JSON 往返稳定性 —— 只测 contract_moisture 曾经让
// moisture_excess_double_threshold 那条分支(删掉照样全绿)偷偷漏测过一次.
describe.each([
  ["contract_moisture", 8] as const,
  ["moisture_excess_double_threshold", 12] as const,
])("mergePurchaseTerms — %s 的三态继承", (field, templateValue) => {
  it("键缺失时继承模板", () => {
    const merged = mergePurchaseTerms(template, { clauses: [] }, { A: 10 });
    expect(merged.terms?.[field]).toBe(templateValue);
  });

  it("键存在但值为 undefined 时视同缺失, 继承模板", () => {
    const merged = mergePurchaseTerms(template, { clauses: [], [field]: undefined }, { A: 10 });
    expect(merged.terms?.[field]).toBe(templateValue);
  });

  it("键存在且值为 null 时显式关闭, 不回退模板", () => {
    const merged = mergePurchaseTerms(template, { [field]: null }, { A: 10 });
    expect(merged.terms?.[field]).toBeNull();
  });

  it("JSON 往返后行为不变(JSON.stringify 会丢掉值为 undefined 的键, 往返前后必须算出同一个结果)", () => {
    const override = { clauses: [], [field]: undefined };
    const before = mergePurchaseTerms(template, override, { A: 10 });
    const roundTripped = JSON.parse(JSON.stringify(override));
    const after = mergePurchaseTerms(template, roundTripped, { A: 10 });
    expect(after.terms?.[field]).toBe(before.terms?.[field]);
    expect(before.terms?.[field]).toBe(templateValue); // 双重确认: 往返前后都是"继承", 不是巧合地都错
  });
});
```

- [ ] **Step 2: 运行确认失败**

```bash
cd doudou_blend && npx vitest run src/penalty.test.ts
```
预期：FAIL —— `Cannot find module './penalty'`。

- [ ] **Step 3: 实现 `doudou_blend/src/penalty.ts`**

```ts
import type { Direction, Penalty, PurchaseClause, PurchaseTerms } from "./types";

/** 模板里的一条条款: 只有条款本身, 保证值由每种煤各自提供. */
export interface PenaltyTemplateClause {
  indicator: string;
  direction: Direction;
  penalty: Penalty;
}

/** 全局扣款模板: 行业通用条款, 所有煤默认套用. */
export interface PenaltyTemplate {
  clauses: PenaltyTemplateClause[];
  contract_moisture?: number | null;
  moisture_excess_double_threshold?: number | null;
}

/** 单煤覆盖: 只列出与模板不同的条款. */
export interface CoalPenaltyOverride {
  clauses?: PenaltyTemplateClause[];
  /**
   * 该煤明确不适用的模板条款指标: 即使有保证值也不产出条款, 且不计入孤儿保证值
   * (这是用户主动排除, 不是数据丢失). 真实采购合同确实会只对部分指标计价.
   */
  excluded_indicators?: string[];
  contract_moisture?: number | null;
  moisture_excess_double_threshold?: number | null;
}

/**
 * 合并结果. `terms` 供 core 使用; `orphanedGuarantees` 标出"有保证值却找不到
 * 条款"的指标 —— 调用方应据此提示用户, 而不是静默按无扣款处理.
 *
 * 孤儿保证值最常见的成因: 全局模板只存 localStorage 不跨设备同步
 * (见 penaltyStorage.ts), 换设备登录后 CoalPref 里的 purchase_guarantees
 * 随 coal_prefs 正常同步过来了, 但模板没有 —— 这种煤本该计价却悄悄零扣款,
 * 两台设备算出的成本会不一致但界面上什么都不会报错。
 */
export interface MergedPurchaseTerms {
  terms: PurchaseTerms | null;
  orphanedGuarantees: string[];
}

/**
 * 把合同原文的"每 step 个单位扣 amount 元"换算成 元/吨·单位.
 * 合同写"每超 0.1% 扣 8 元/吨" ⇒ tierRate({ step: 0.1, amount: 8 }) = 80.
 * 让用户照抄合同, 不做心算 —— 8 与 80 差一个量级。
 *
 * 用具名对象参数, 不用位置参数: `tierRate(8, 0.1)` 这种参数顺序传反的调用,
 * 位置参数会静默算出一个看似合理但错误的数字, 对象参数会直接类型对不上/
 * 值域检查失败, 编译期或运行期都能截住。
 *
 * 非法输入(step/amount 非有限数、step<=0、amount<0)返回 null, 不是 0 ——
 * 0 本身是合法的零费率档位(某档不扣钱), 不能拿它当错误信号, 调用方必须
 * 显式处理 null。
 */
export function tierRate({
  step,
  amount,
}: {
  step: number;
  amount: number;
}): number | null {
  if (!Number.isFinite(step) || step <= 0) return null;
  if (!Number.isFinite(amount) || amount < 0) return null;
  return amount / step;
}

/**
 * 合并全局模板与单煤覆盖, 配上该煤的保证值, 产出 core 需要的 PurchaseTerms。
 *
 * 规则:
 *   - 覆盖按指标逐条替换模板条款(不是整体替换); `excluded_indicators` 里的
 *     指标即使在模板或覆盖里有条款也不产出(实现"单煤覆盖压制模板条款",
 *     而不仅是新增/替换)。
 *   - 没有保证值的指标不产出条款(没有保证值就无从判定偏离), 也不算孤儿
 *     (这只是"没配置", 不是"配置丢了")。
 *   - 有保证值却在模板+覆盖里都找不到条款的指标(且未被显式排除), 计入
 *     `orphanedGuarantees` —— 调用方应提示用户, 而不是当作用户没配置。
 *   - `contract_moisture` / `moisture_excess_double_threshold`: 覆盖对象里
 *     这个键为具体数字就用它; 为 `null` 是显式关闭, 不回退模板; 键不存在
 *     或值为 `undefined` 都视为"没设置", 回退模板。
 *     "键不存在"与"值为 undefined"必须同等对待(而不是用 `in` 单独区分),
 *     因为 `JSON.stringify` 会丢弃值为 undefined 的键 —— 区分它们会让
 *     同一个覆盖对象在写入/读出 localStorage 前后表现不同, 用户能看到的
 *     症状是"配方价格随页面刷新变化"。
 *   - `terms === null` 表示该煤没有任何可用采购条款, 调用方应省略
 *     `purchase_terms` 字段。
 */
export function mergePurchaseTerms(
  template: PenaltyTemplate | null,
  override: CoalPenaltyOverride | undefined,
  guarantees: Partial<Record<string, number>>,
): MergedPurchaseTerms {
  const byIndicator = new Map<string, PenaltyTemplateClause>();
  for (const clause of template?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }
  for (const clause of override?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }
  const excluded = new Set(override?.excluded_indicators ?? []);

  const clauses: PurchaseClause[] = [];
  const orphanedGuarantees: string[] = [];
  for (const [indicator, guarantee] of Object.entries(guarantees)) {
    if (typeof guarantee !== "number" || !Number.isFinite(guarantee)) continue;
    if (excluded.has(indicator)) continue;
    const clause = byIndicator.get(indicator);
    if (!clause) {
      orphanedGuarantees.push(indicator);
      continue;
    }
    clauses.push({
      indicator,
      direction: clause.direction,
      guarantee,
      penalty: clause.penalty,
    });
  }

  // 没有任何条款落地 = 该煤未配置采购扣款, 即使模板/覆盖带了合同水分也不该
  // 套用(水分双倍计入是"该煤有采购合同"的推论, 不该在没有条款时生效)。
  if (clauses.length === 0) {
    return { terms: null, orphanedGuarantees };
  }

  const contract_moisture = inheritableMoistureField(
    override?.contract_moisture,
    template?.contract_moisture,
  );
  const moisture_excess_double_threshold = inheritableMoistureField(
    override?.moisture_excess_double_threshold,
    template?.moisture_excess_double_threshold,
  );

  return {
    terms: { clauses, contract_moisture, moisture_excess_double_threshold },
    orphanedGuarantees,
  };
}

/**
 * `contract_moisture` / `moisture_excess_double_threshold` 共用的继承规则:
 * 覆盖值是具体数字就用它; 是 `null` 就显式关闭(不回退模板); 是 `undefined`
 * (无论键是否存在, 两者在 JS 里读取结果相同)就当没设置, 回退模板的值。
 */
function inheritableMoistureField(
  overrideValue: number | null | undefined,
  templateValue: number | null | undefined,
): number | null {
  return overrideValue !== undefined ? overrideValue : (templateValue ?? null);
}
```

- [ ] **Step 4: 运行测试**

```bash
cd doudou_blend && npx vitest run src/penalty.test.ts
```
预期：全部 PASS。

- [ ] **Step 5: 实现模板存储 `doudou_blend/src/penaltyStorage.ts`**

```ts
import type { PenaltyTemplate } from "./penalty";

// 只存 localStorage, 不进 UserStorageSnapshot ——
// 服务端用户存储是显式 PostgreSQL 列(coal_prefs/contract/quantity/user_coals),
// 新增顶层键需要建表迁移, 不在本期范围. 代价: 模板不跨设备同步.
const KEY_PENALTY_TEMPLATE = "doudou_blend.penalty_template.v1";

/**
 * 模板变更事件名。订阅方用这个常量, 不要手打字符串字面量 ——
 * 打错字不会报错, 只会让那个屏幕永远收不到刷新事件。
 */
export const PENALTY_TEMPLATE_EVENT = "doudou:penalty_template_changed";

export function getPenaltyTemplate(): PenaltyTemplate | null {
  try {
    const raw = localStorage.getItem(KEY_PENALTY_TEMPLATE);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PenaltyTemplate;
    if (!parsed || !Array.isArray(parsed.clauses)) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function setPenaltyTemplate(template: PenaltyTemplate): void {
  localStorage.setItem(KEY_PENALTY_TEMPLATE, JSON.stringify(template));
  window.dispatchEvent(new CustomEvent(PENALTY_TEMPLATE_EVENT));
}

export function clearPenaltyTemplate(): void {
  localStorage.removeItem(KEY_PENALTY_TEMPLATE);
  window.dispatchEvent(new CustomEvent(PENALTY_TEMPLATE_EVENT));
}
```

- [ ] **Step 6: `CoalPref` 增加买入侧字段**

`doudou_blend/src/storage.ts` 的 `CoalPref` 接口末尾（`updated_at` 之前）插入：

```ts
  /** 该煤采购合同的保证值: 指标 → 保证值. 缺项不算扣款. */
  purchase_guarantees?: Partial<Record<string, number>>;
  /** 该煤与全局扣款模板不同的条款; 缺失 = 完全套用模板. */
  purchase_override?: CoalPenaltyOverride;
```

文件顶部加 import：

```ts
import type { CoalPenaltyOverride } from "./penalty";
```

- [ ] **Step 7: 运行前端测试**

```bash
cd doudou_blend && npm test && npm run build
```
预期：全部 PASS，构建成功。

- [ ] **Step 8: 提交**

```bash
git add doudou_blend/src/penalty.ts doudou_blend/src/penalty.test.ts doudou_blend/src/penaltyStorage.ts doudou_blend/src/storage.ts
git commit -m "feat(penalty): 前端扣款单位换算与模板合并"
```

**与最初设计的差异（两轮质量审查发现, 已修订到上面的代码块里）：**
1. `mergePurchaseTerms` 从返回 `PurchaseTerms | null` 改为返回
   `MergedPurchaseTerms { terms, orphanedGuarantees }`。原因: 模板只存
   localStorage 不跨设备同步, 换设备后"这个煤没配置扣款"和"配置了但模板
   丢了"在旧签名下都表现为 `null`, 两台设备算出的成本会静默不一致。
2. `CoalPenaltyOverride` 新增 `excluded_indicators?: string[]`, 合并逻辑
   末尾按它删除已产出的条款。原因: 原逻辑只能"新增/替换"模板条款, 无法
   表达"这份采购合同压根不管这项指标", 用户只能删保证值迂回, 而保证值
   可能还有其他用途。排除比覆盖条款优先级更高(同一指标两者都给了, 以排除
   为准), 因为排除是更具体的信号: 覆盖条款说明"这项按这个价算", 排除说明
   "这项压根不适用"。
3. `tierRate` 从 `(step, amount): number` 改成
   `({ step, amount }): number | null`。原因: 位置参数传反(`tierRate(amount, step)`)
   会静默算出一个错的但貌似合理的数字, 而这是全前端最关键的一个单位换算;
   返回 `0` 也没法跟"合法的零费率档位"区分, 改成 `null` 显式表达非法输入,
   同时把负 `amount`(倒贴钱的扣款)也纳入非法输入。
4. `penaltyStorage.ts` 用 `CustomEvent` 而非 `Event`(跟仓库其余事件风格
   一致), 并导出 `PENALTY_TEMPLATE_EVENT` 常量供订阅方引用, 避免手打
   字符串字面量打错字导致订阅永远失效。
5. `contract_moisture` / `moisture_excess_double_threshold` 的继承判断从
   `"key" in override`(区分"键不存在"与"键存在但值为 undefined")改成只看
   `override[key] !== undefined`(两者同等对待, 都算"没设置", 回退模板)。
   原因: `JSON.stringify` 会丢弃值为 undefined 的键, 用 `in` 区分这两种
   情况, 会让同一个覆盖对象在写入/读出 localStorage 前后算出不同的结果——
   页面刷新一下, 配方价格就变了, 且完全没有报错信号。**Task 7/8 因此不必
   小心避免往这两个键里 spread 出 `undefined`**, 这个坑已经在这一层堵死。

---

### Task 7: 今日屏成本卡（含 `TodayScreen.tsx` 拆分）

**Files:**
- Create: `doudou_blend/src/screens/CostCard.tsx`
- Modify: `doudou_blend/src/screens/TodayScreen.tsx`（916 行 → 抽出成本卡）
- Modify: `doudou_blend/src/backend.ts`（历史净成本派生，见 Step 0）
- Test: `doudou_blend/src/screens/CostCard.test.tsx`、`doudou_blend/src/backend.test.ts`

- [ ] **Step 0: 历史记录显示的是报价而非净成本（Task 5 排查发现）**

`blend_kit_server/src/database.rs:236` 把 `/cost/cif_per_ton` 抽成 `cost_cif` 列，
`HistoryScreen.tsx:274` 直接渲染 `¥{entry.cost_cif.toFixed(2)}`。启用合同条款后，
这个数字就不再是真实成本——买入侧折扣和卖出侧扣款都没算进去。

**不改服务端、不加列、不做迁移。** 按仓库既有分层（Rust 侧存不透明 blob，
TS adapter 从 result 派生展示字段）在 `backend.ts` 的 history adapter 里派生：
优先取 `result.cost.net_per_ton`，缺失时回退到 `cost_cif` 列。

老记录的 blob 里没有 `net_per_ton`，回退能生效正是因为 Task 1 把这四个字段
在 TS 侧标成了可选；不要把它们改回必填。

测试：带 `net_per_ton` 的记录取净成本；不带的老记录回退到 `cost_cif`。

- [ ] **Step 1: 写失败测试**

创建 `doudou_blend/src/screens/CostCard.test.tsx`：

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CostCard } from "./CostCard";
import type { CostBreakdown } from "../types";

const base: CostBreakdown = {
  fob_per_ton: 1000,
  frt_per_ton: 100,
  cif_per_ton: 1100,
  total_fob: null,
  total_frt: null,
  total_cif: null,
  purchase_adjust_per_ton: 0,
  penalty_per_ton: 0,
  net_per_ton: 1100,
  total_purchase_adjust: null,
  total_penalty: null,
  total_net: null,
};

describe("CostCard", () => {
  it("无扣款时只显示到厂价与净成本", () => {
    render(<CostCard cost={base} />);
    expect(screen.getByText("到厂价")).toBeInTheDocument();
    expect(screen.queryByText("预计扣款")).not.toBeInTheDocument();
    expect(screen.queryByText("买入修正")).not.toBeInTheDocument();
  });

  it("有卖出扣款时显示扣款行", () => {
    render(<CostCard cost={{ ...base, penalty_per_ton: 64, net_per_ton: 1164 }} />);
    expect(screen.getByText("预计扣款")).toBeInTheDocument();
    expect(screen.getByText("64.00 元/吨")).toBeInTheDocument();
    expect(screen.getByText("1164.00 元/吨")).toBeInTheDocument();
  });

  it("有买入修正时显示修正行, 折扣为负值", () => {
    render(
      <CostCard cost={{ ...base, purchase_adjust_per_ton: -40, net_per_ton: 1060 }} />,
    );
    expect(screen.getByText("买入修正")).toBeInTheDocument();
    expect(screen.getByText("-40.00 元/吨")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 运行确认失败**

```bash
cd doudou_blend && npx vitest run src/screens/CostCard.test.tsx
```
预期：FAIL —— `Cannot find module './CostCard'`。

- [ ] **Step 3: 实现 `doudou_blend/src/screens/CostCard.tsx`（已按实现修订，见下方"实际交付"说明）**

⚠ **给下一个读这份计划的人**：下面这版是 Task 7 执行时发现原计划有问题后，
经 team-lead 确认修订过的版本，不是最初草稿。原计划的 Step 4 写的是"整段替换"
`TodayScreen.tsx` 里渲染 `cost.cif_per_ton` 的成本区块 —— 但那个区块（原
`.cost-card.today-cost` div）不只是价格数字，还打包了 `.cost-meta`（数值界内
/质量/可选煤数/总额徽章）和 `.cost-drift`（报价时效/锚点推算/现货指数提示）。
`.cost-drift` 有自己的一组回归测试（`TodayScreen.test.tsx` 的
"TodayScreen 报价时效提示" describe block），代码里还留了一条注释明确警告过
"真正危险的是没推算的默认态，不能不提示" —— 照原计划"整段替换 + 删掉
formatPrice"会连这部分一起删掉。**执行时发现后立即停下汇报，不要自己删。**

实际交付的设计：

- `.cost-meta` / `.cost-drift` **完全不动**，还留在 `TodayScreen.tsx` 里，只是从
  `.cost-card.today-cost` 搬进了一个新的同级"状态卡"（`.cost-card.cost-status-card`），
  和 `<CostCard>` 一起包在 `<div className="today-cost">` 里（两张蓝卡上下堆叠，
  `.today-cost` 仍是 `.today-dashboard` 网格里那一格）。
- `CostCard` 的大字号主位（`.cost-int`/`.cost-dec`/`.cost-unit` 两段式，
  `formatPrice` 也从 `TodayScreen.tsx` **搬进**（不是删除）`CostCard.tsx`）
  展示的是 `net_per_ton`（回退 `cif_per_ton`），标签仍是"最低到厂价"不变 ——
  净成本才是 LP 实际求最优的数字，到厂价条款生效后只是报价，大字号主位必须
  跟着净成本走，否则这个功能要展示的数字反而被继续藏起来。没有计价条款时
  `net_per_ton === cif_per_ton`，大字号数值和老界面完全一致，界面观感不变。
- 大字号下方是明细行：到厂价（`cost.cif_per_ton`，恒定展示）、买入修正、
  预计扣款（后两行 `Math.abs(...) > 1e-6` 时才显示，值为 0/缺失就隐藏）。
- 额外加了 Step 8（孤儿保证值告警，见下方），`orphanedGuarantees` 非空时在卡片
  顶部插入 `role="alert"` 的红色告警块，点名煤种和指标。

```tsx
import { INDICATOR_LABEL } from "../types";
import type { CostBreakdown } from "../types";
import type { OrphanedGuaranteeWarning } from "../domain/resolvedCoal";

const YUAN = (value: number) => `${value.toFixed(2)} 元/吨`;

/** 拆成整数/小数两段, 配合 .cost-int/.cost-dec 两段式大字号. */
function formatPrice(n: number): { int: string; dec: string } {
  const [intPart, decPart] = n.toFixed(2).split(".");
  return { int: intPart, dec: decPart };
}

function indicatorLabel(indicator: string): string {
  return INDICATOR_LABEL[indicator] ?? indicator;
}

export function CostCard({
  cost,
  orphanedGuarantees = [],
}: {
  cost: CostBreakdown;
  orphanedGuarantees?: OrphanedGuaranteeWarning[];
}) {
  const hasAdjust = Math.abs(cost.purchase_adjust_per_ton ?? 0) > 1e-6;
  const hasPenalty = Math.abs(cost.penalty_per_ton ?? 0) > 1e-6;
  const netPerTon = cost.net_per_ton ?? cost.cif_per_ton;
  const { int: costInt, dec: costDec } = formatPrice(netPerTon);

  return (
    <div className="cost-card">
      {orphanedGuarantees.length > 0 && (
        <div className="cost-orphan-warning" role="alert">
          {/* ⚠ 采购扣款模板在本设备缺失, 点名煤种/指标 —— 见 Step 8 */}
        </div>
      )}
      <div className="cost-label">最低到厂价</div>
      <div className="cost-amount">
        <span className="cost-int">{costInt}</span>
        <span className="cost-dec">.{costDec}</span>
        <span className="cost-unit">元/吨</span>
      </div>
      <div className="cost-row">
        <span>到厂价</span>
        <span>{YUAN(cost.cif_per_ton)}</span>
      </div>
      {hasAdjust && (
        <div className="cost-row cost-row--adjust">
          <span>买入修正</span>
          <span>{YUAN(cost.purchase_adjust_per_ton ?? 0)}</span>
        </div>
      )}
      {hasPenalty && (
        <div className="cost-row cost-row--penalty">
          <span>预计扣款</span>
          <span>{YUAN(cost.penalty_per_ton ?? 0)}</span>
        </div>
      )}
    </div>
  );
}
```

（完整实现含 data-testid 与告警块的具体文案，见仓库里的
`doudou_blend/src/screens/CostCard.tsx`。）

- [ ] **Step 4: 在 `TodayScreen.tsx` 换用 `CostCard`（范围比原计划窄 —— 见上方警告）**

**不要**整段删掉 `.cost-card.today-cost`。只把原来渲染 `cost.cif_per_ton` 大字号
（`cost-label` + `cost-amount` 那一小块，连同局部的 `formatPrice`/`costInt`/`costDec`）
换成 `<CostCard cost={cost} orphanedGuarantees={...} />`；`.cost-meta` 和
`.cost-drift`（报价时效提示）原样保留，和 `<CostCard>` 一起放进
`<div className="today-cost">` 包裹的两张蓝卡里（详见上方"实际交付"）。
`formatPrice` 搬进 `CostCard.tsx`，不要在 `TodayScreen.tsx` 里删掉后重新定义。

在文件顶部加 `import { CostCard } from "./CostCard";`。

**因为大字号主位换成了 `net_per_ton` 但没有配置计价条款时数值和原来的
`cif_per_ton` 完全相等，`TodayScreen.test.tsx` 里原有的 `.cost-int` 选择器断言
不需要改也能继续通过** —— 不要为了"用新组件"去重写这些断言，那样会造成没必要
的测试改动量，也更容易在改的过程中悄悄削弱覆盖。

- [ ] **Step 5: 指标体检显示扣款额**

在 `TodayScreen.tsx` 渲染 `indicator_check` 的行内，把状态标记逻辑改为：

```tsx
{check.penalty_per_ton != null && check.penalty_per_ton > 1e-6 ? (
  <span className="indicator-penalty">扣 {check.penalty_per_ton.toFixed(2)} 元/吨</span>
) : (
  /* 既有的 Pass/Fail 标记保持不变 */
  existingStatusBadge
)}
```

这一步和原计划一致, 没有变化.

- [ ] **Step 8: 采购扣款模板缺失告警（不在原计划里, Task 6 复盘后新加）**

`mergePurchaseTerms`（`penalty.ts`）返回 `orphanedGuarantees`：一种煤有
`purchase_guarantees` 却在模板+覆盖里都找不到匹配条款的指标列表。最常见成因是
全局扣款模板只存 localStorage 不跨设备同步（见 `penaltyStorage.ts` 顶部注释），
换设备登录后煤的保证值随 `coal_prefs` 同步过来了但模板没有 —— 这种煤本该计价
却被静默当无扣款处理，两台设备算出的成本不一致但界面上什么都不会报错。

新增 `collectOrphanedGuarantees(pool, prefs, template)`（`domain/resolvedCoal.ts`）：
扫描 `effectiveEnabled` 的煤（停用/隐藏的煤不用吓用户），逐个调用
`mergePurchaseTerms` 收集非空的 `orphanedGuarantees`，产出
`{coal, indicators}[]`。在 `TodayScreen.tsx` 的 `runSolve` 里算好后存进
`SolveSnapshot.orphanedGuarantees`（新增的可选字段, `domain/solveSession.ts`），
传给 `<CostCard>` 的同名 prop；非空时卡片顶部插入 `role="alert"` 的红色告警块，
点名煤种和指标（中文标签走 `INDICATOR_LABEL`）。另外监听了
`PENALTY_TEMPLATE_EVENT`（跟已有的 prefs/contract/user_coals 变化监听对称），
模板一变就自动重算刷新告警。

- [ ] **Step 6: 运行测试**

```bash
cd doudou_blend && npm test && npm run build
```
预期：`CostCard.test.tsx` PASS，`TodayScreen.test.tsx` 无回归。若 `TodayScreen.test.tsx`
因构造 `CostBreakdown` 缺新字段而失败，补上 Task 1 新增的 6 个字段。

**实际执行时额外发现并顺手修的两处同类缺陷**（导出/展示用了报价而非净成本，
和 Step 0 是同一类问题，不在原计划范围内，但值得记录避免以后重犯）：
`buildOrderText`（"导出订单"剪贴板文本）原来全程用 `cif_per_ton`/`cif_amount`/
`total_cif`；`.cost-meta` 里的"总额"徽章也一样。两处都改成优先
`net_per_ton`/`cif_eff_amount`/`total_net`，缺失时回退报价字段。

- [ ] **Step 7: 提交**

```bash
git add doudou_blend/src/screens/CostCard.tsx doudou_blend/src/screens/CostCard.test.tsx doudou_blend/src/screens/TodayScreen.tsx doudou_blend/src/screens/TodayScreen.test.tsx doudou_blend/src/backend.ts doudou_blend/src/backend.test.ts doudou_blend/src/domain/resolvedCoal.ts doudou_blend/src/domain/resolvedCoal.test.ts doudou_blend/src/domain/solveSession.ts doudou_blend/src/App.css
git commit -m "feat(today): 成本卡拆出独立组件, 展示买入修正/卖出扣款, 并告警扣款模板缺失"
```

---

### Task 8: 合同屏与煤池屏录入

**Files:**
- Modify: `doudou_blend/src/screens/ContractScreen.tsx`
- Modify: `doudou_blend/src/screens/CoalPoolScreen.tsx`

> `CoalPenaltyOverride` 的 `contract_moisture` / `moisture_excess_double_threshold`
> 不必小心避免 spread 出 `undefined`（例如 `{ ...prevOverride, contract_moisture: form.moisture || undefined }`
> 这类写法是安全的）：`mergePurchaseTerms` 把"键不存在"和"值为 undefined"
> 同等对待, 两者都回退模板; 只有显式传 `null` 才会关闭该项。这个坑已经在
> Task 6 堵死, 不需要本任务的表单逻辑操心。

- [ ] **Step 1: 合同屏 —— 每条 spec 的计价开关**

在 `ContractScreen.tsx` 每条 spec 行下方新增可折叠区块。状态更新函数：

```tsx
function togglePriced(spec: Spec, on: boolean): Spec {
  if (!on) return { ...spec, enforcement: "Hard", penalty: null };
  const bound = spec.direction === "Upper" ? spec.max : spec.min;
  const fallback = spec.direction === "Upper" ? (bound ?? 0) * 1.1 : (bound ?? 0) * 0.9;
  return {
    ...spec,
    enforcement: "Priced",
    penalty: spec.penalty ?? { tiers: [{ rate: 0 }], reject: fallback },
  };
}
```

区间型指标禁用开关（core 会报错）：

```tsx
<label>
  <input
    type="checkbox"
    disabled={spec.direction === "Range"}
    checked={spec.enforcement === "Priced"}
    onChange={(event) => update(togglePriced(spec, event.target.checked))}
  />
  计价（超界不判不合格，按合同扣款折算成成本）
</label>
```

- [ ] **Step 2: 合同屏 —— 档位表按合同原文两栏录入**

每档一行，用户照抄合同的"每 0.1%"与"扣 8 元"，前端换算：

```tsx
{spec.penalty?.tiers.map((tier, index) => (
  <div key={index} className="tier-row">
    每
    <input
      type="number"
      step="0.01"
      value={tierSteps[index] ?? 0.1}
      onChange={(event) => setTierStep(index, Number(event.target.value))}
    />
    {spec.indicator === "G" || spec.indicator === "CSR" ? "点" : "%"}
    扣
    <input
      type="number"
      value={tierAmounts[index] ?? 0}
      onChange={(event) => {
        setTierAmount(index, Number(event.target.value));
        const rate = tierRate({ step: tierSteps[index] ?? 0.1, amount: Number(event.target.value) });
        // tierRate 对非法输入(负数/非有限/step<=0)返回 null, 不是 0 ——
        // 0 是合法的零费率档位, 不能用它吞掉错误输入; null 时不写回, 保留旧值。
        if (rate != null) updateTier(index, { ...tier, rate });
      }}
    />
    元/吨
    {index < (spec.penalty?.tiers.length ?? 0) - 1 && (
      <>
        ，本档覆盖
        <input
          type="number"
          step="0.01"
          value={tier.width ?? 0}
          onChange={(event) => updateTier(index, { ...tier, width: Number(event.target.value) })}
        />
      </>
    )}
  </div>
))}
<button onClick={addTier}>加一档（更高扣款率）</button>
```

`tierSteps` / `tierAmounts` 是组件内 `useState` 的展示态数组，只用于回显用户录入的原文；
存进 `Spec.penalty` 的永远是换算后的 `rate`。

- [ ] **Step 3: 合同屏 —— 拒收线输入与前端预校验**

```tsx
<label>
  拒收线（越过即不接受，不再计价）
  <input
    type="number"
    value={spec.penalty?.reject ?? 0}
    onChange={(event) =>
      update({ ...spec, penalty: { ...spec.penalty!, reject: Number(event.target.value) } })
    }
  />
</label>
{penaltyError(spec) && <p className="form-error">{penaltyError(spec)}</p>}
```

```tsx
/** 与 Rust validate_penalty 同规则的前端预检, 提前给出人话提示. */
function penaltyError(spec: Spec): string | null {
  const penalty = spec.penalty;
  if (spec.enforcement !== "Priced" || !penalty) return null;
  if (penalty.tiers.length === 0) return "至少要有一档扣款";
  for (let index = 1; index < penalty.tiers.length; index += 1) {
    if (penalty.tiers[index].rate <= penalty.tiers[index - 1].rate) {
      return `第 ${index + 1} 档扣款率必须高于前一档`;
    }
  }
  if (penalty.tiers[penalty.tiers.length - 1].width != null) return "末档不能有宽度上限";
  if (spec.direction === "Upper" && penalty.reject < (spec.max ?? 0)) {
    return "拒收线必须不低于合同上限";
  }
  if (spec.direction === "Lower" && penalty.reject > (spec.min ?? 0)) {
    return "拒收线必须不高于合同下限";
  }
  return null;
}
```

顶部加 `import { tierRate } from "../penalty";`。

- [ ] **Step 4: 煤池屏 —— 单煤保证值录入**

在 `CoalPoolScreen.tsx` 每种煤的化验覆盖区块旁，新增保证值输入。写回 `CoalPref`：

```tsx
setCoalPref(coal.name, {
  purchase_guarantees: { ...(pref?.purchase_guarantees ?? {}), [indicator]: value },
});
```

- [ ] **Step 5: 煤池屏 —— 全局模板入口**

屏顶加一个"采购扣款模板"折叠区，复用 Step 2 的档位录入 UI，读写
`getPenaltyTemplate()` / `setPenaltyTemplate()`。顶部加：

```tsx
import { getPenaltyTemplate, setPenaltyTemplate } from "../penaltyStorage";
```

- [ ] **Step 6: 组装请求时挂上 `purchase_terms`**

找到构造 `Coal[]` 送给 `solveJson` 的位置（`TodayScreen.tsx` 或其调用的 helper），
为每种煤补：

```tsx
const { terms: purchase_terms, orphanedGuarantees } = mergePurchaseTerms(
  getPenaltyTemplate(),
  pref?.purchase_override,
  pref?.purchase_guarantees ?? {},
);
// orphanedGuarantees 非空 = 该煤有保证值却找不到匹配条款(最常见成因:
// 换设备登录, CoalPref 里的 purchase_guarantees 随 coal_prefs 同步过来了,
// 但全局模板只存本机 localStorage, 没有同步) —— 对应的界面警示在 Task 7
// (今日屏成本卡) 里处理, 这里只负责把它算出来, 不要在这一步吞掉。
```

再把 `purchase_terms` 填进对应煤的请求对象。

- [ ] **Step 7: 运行测试与构建**

```bash
cd doudou_blend && npm test && npm run build
```
预期：全部 PASS。

- [ ] **Step 8: 提交**

```bash
git add doudou_blend/src/screens/ContractScreen.tsx doudou_blend/src/screens/CoalPoolScreen.tsx doudou_blend/src/screens/TodayScreen.tsx
git commit -m "feat(contract): 合同屏计价条款录入与煤池屏采购保证值"
```

---

## Phase 3 — 验证（Task 9）

### Task 9: 全量验证与端到端自检

- [ ] **Step 1: 核心算法全量**

```bash
cd blend_kit_rs && cargo test --release && cargo clippy --release -- -D warnings && cargo fmt --check
cargo run --release --example master_demo
cargo run --release --example demo
```
预期：测试全绿，两个 example 正常输出。

- [ ] **Step 2: 服务端未受影响**

```bash
cd blend_kit_server && cargo test --locked && cargo clippy --release -- -D warnings
```
预期：全绿（本计划未改服务端；此步是确认 `blend_kit` 的类型变更没有连累它）。

- [ ] **Step 3: 前端全量与一致性**

```bash
cd doudou_blend && npm test && npm run build && npm run check:consistency && npm run check:data
```
预期：全绿。`check:consistency` 特别要确认 `Enforcement` 枚举含 4 个变体、
`Coal`/`Spec`/`CostBreakdown`/`OrderItem`/`IndicatorCheck` 字段两边一致。

- [ ] **Step 4: 用合同真实数字做一次端到端手验**

启动本地服务与前端，在合同屏按本合同录入：灰 ≤10% 计价 80 元/吨·%、拒收 11.5；
粘结 ≥85 计价 5 元/吨·点、拒收 80。求解后确认：

- 成本卡出现"预计扣款"与"净成本"两行
- 粘结指标显示"扣 N 元/吨"而非红色不合格
- 配方相对全硬约束时更便宜（净成本更低）

- [ ] **Step 5: 完整变更审查**

```bash
cd /Users/lyf/dev/coalassistant && git diff main...HEAD --stat
```
逐文件核对无遗留调试代码、无 `TODO`。

- [ ] **Step 6: 提交收尾**

```bash
git add -A && git commit -m "test: 合同扣款建模全量验证" || echo "无待提交改动"
```

---

## 自检记录（写计划时执行）

**规格覆盖：** 设计文档 A~G 七节均有对应任务 —— A→Task 1、B→Task 2、C→Task 3+4+5、
D→Task 1(字段)+4/5(填值)、E→Task 6~8、F→各任务测试步、G→Task 9。
"不在范围内"四项（逾期 30 元/吨、违约金 25%、结算台阶量化、CSR 回归接入）无任务，符合预期。

**与设计文档的两处偏差（需用户知悉）：**
1. 设计文档 E 节说模板"存储走 `storage.ts` / `cloudStorage.ts`"。实测服务端用户存储是
   PostgreSQL 显式列，新增顶层键需迁移。改为模板只存 localStorage，**不跨设备同步**；
   单煤保证值与覆盖存 `CoalPref`（走既有 `coal_prefs` 列，正常同步）。
2. 设计文档 G 节的三步里，`types.ts` 镜像原属第 2 步；因 CI 逐字段比对，已前移进 Task 1。

**类型一致性：** `Penalty` / `PenaltyTier` / `PurchaseClause` / `PurchaseTerms` 在
Rust（Task 1 Step 3）、TS（Task 1 Step 8）、前端工具（Task 6 Step 3）三处字段名一致；
`tiered_amount` / `effective_cif` 仅在 `penalty.rs` 定义并由 `optimizer.rs` 调用；
`tierRate` / `mergePurchaseTerms` 仅在 `penalty.ts` 定义，由 Task 8 Step 2/6 调用。
