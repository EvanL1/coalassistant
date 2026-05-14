# blend_kit_rs - 豆哥配煤 Rust 核心算法

豆哥配煤 APP 的配煤优化引擎, 基于 Clarabel LP 求解器, 把 8 项煤质指标约束 + 最低到厂成本目标转化为线性规划并求解.

## 8 项指标

| 标识 | 中文 | 来源 | 性质 |
|------|------|------|------|
| S | 硫 | 化验单 (质检) | 越低越好 |
| A | 灰 | 化验单 (质检) | 越低越好 |
| V | 挥发 | 化验单 (质检) | 目标范围 |
| G | 粘结 | 化验单 (质检) | 越高越好 |
| Y | 胶质 | 化验单 (质检) | 越高越好 |
| petro | 岩相 | 经验值 (技术) | 目标范围 |
| CSR | 焦炭强度 | 经验值 (技术) | 越高越好 |
| M | 水分 | 化验单 (质检) | 越低越好 |

8 项指标在 LP 中全部按线性加权处理. petro 和 CSR 作为经验值由技术部门 per-coal 直接录入, 不做公式预测.

## 数据流程

```
业务侧 4 张表
  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐
  │  化验单      │  │  合同约束    │  │  煤价报价    │  │  物流报价    │
  │ S/A/V/G/Y/M │  │  Spec 列表  │  │  FOB (元/吨) │  │  FRT (元/吨) │
  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘
         │                │                │                │
         └────────────────┴────────────────┴────────────────┘
                                   │
                            BlendRequest
                          (COALS + SPECS)
                                   │
                          LP 建模 (Clarabel)
                     min Σ cif(i)·xᵢ   s.t. Σ xᵢ = 1
                     8 项指标线性加权约束
                                   │
                             后处理
                    ┌──────────────┼──────────────┐
                    │              │              │
              视图 A          视图 B         视图 C
           CostBreakdown    OrderItem[]  IndicatorCheck[]
             (财务)           (采购)        (质检/销售)
```

## 核心 API 示例

```rust
use blend_kit::{coal_from_tuple, solve, BlendRequest, Spec};

fn main() {
    // 构造煤池: coal_from_tuple(名称, (S, A, V, G, Y, petro, CSR, M, FOB, FRT))
    let coals = vec![
        coal_from_tuple("临北", (2.0, 6.0, 22.0, 93.0, 17.0, 0.01, 70.0, 11.0, 1425.0, 25.0)),
        // ... 更多煤
    ];

    // 合同约束
    let specs = vec![
        Spec::upper("S", 2.5),         // 硫 ≤ 2.5
        Spec::upper("A", 9.0),         // 灰 ≤ 9.0
        Spec::range("V", 18.0, 27.0),  // 挥发 18~27
        Spec::lower("G", 80.0),        // 粘结 ≥ 80
        Spec::lower("Y", 14.0),        // 胶质 ≥ 14
        Spec::upper("M", 12.0),        // 水分 ≤ 12
        Spec::lower("CSR", 60.0),      // 焦炭强度 ≥ 60
    ];

    let req = BlendRequest {
        coals,
        specs,
        total_quantity: Some(3700.0), // 总采购吨数, None 则只输出比例
        truncate_decimal: true,
    };

    let r = solve(&req);

    if r.ok {
        // 视图 A: 成本结构
        let cost = r.cost.unwrap();
        println!("FOB: {:.2} 元/吨", cost.fob_per_ton);
        println!("FRT: {:.2} 元/吨", cost.frt_per_ton);
        println!("CIF: {:.2} 元/吨", cost.cif_per_ton);

        // 视图 B: 实物订单
        for o in &r.orders {
            println!("{}: {:.1}% / {:.2} 吨", o.coal, o.ratio * 100.0, o.tons.unwrap_or(0.0));
        }

        // 视图 C: 指标体检 (含 binding 标记)
        for ic in &r.indicator_check {
            println!("{} = {:.2}  binding={}", ic.label_zh, ic.value, ic.binding);
        }
    }
}
```

临北单煤数据 (对应 `test_linbei_tuple`): S=2.0, A=6.0, V=22, G=93, Y=17, petro=0.01, CSR=70, M=11, FOB=1425, FRT=25, cif()=1450.

## JSON API

移动端通过 `solve_json` 调用:

```rust
let output_json: String = solve_json(input_json);
```

入参: `BlendRequest` 的 JSON 序列化. 出参: `BlendResult` 的 JSON 序列化.

详细 JSON Schema 见 `docs/JSON_SCHEMA.md`.

## 三视图说明

### 视图 A - CostBreakdown (给财务)

| 字段 | 说明 |
|------|------|
| fob_per_ton | 加权 FOB 出厂价, 元/吨 |
| frt_per_ton | 加权 FRT 运费, 元/吨 |
| cif_per_ton | 加权到厂价 = FOB + FRT |
| total_fob / total_frt / total_cif | 提供 total_quantity 时填充 |

CIF 是派生量, 不存储为字段, 由 `Coal::cif()` 函数计算: `fob + frt`.

### 视图 B - OrderItem 列表 (给采购)

| 字段 | 说明 |
|------|------|
| coal | 煤名 |
| ratio | LP 最优配比 xᵢ* |
| tons | 实物吨数 (total_quantity × ratio) |
| fob_amount / frt_amount / cif_amount | 各项金额 |

按配比降序排列, 只含配比 > 1e-5 的煤.

### 视图 C - IndicatorCheck 列表 (给质检/销售)

| 字段 | 说明 |
|------|------|
| indicator | 指标标识 (S/A/V 等) |
| label_zh | 中文标签 |
| value | 混合后实际值 |
| min / max | 合同约束边界 |
| slack | 距最近边界余量 (`Option<f64>`). `None` 表示无约束, 负值表示违反 |
| binding | 是否顶格 (slack 接近 0 且非负, 即真正卡到合同边界) |

按 `INDICATORS` 固定顺序输出.

## binding 检测与谈判方向

LP 求解后, 对每条约束计算 slack. `slack ≈ 0` 的约束标记为 binding, 即当前解已把该指标"顶格". binding 集合是谈判方向的逆向归因依据:

```
S 顶格 (binding=true, max=2.5)
  → 解已把硫用满. 降本空间被硫限制住.
  → 谈判方向: 找更低硫煤, 或与客户谈宽硫上限

G 顶格 (binding=true, min=80)
  → 粘结指数恰好踩线.
  → 谈判方向: 找更高粘结煤, 或谈松粘结下限
```

## 容错规则

- `Spec::enabled = false`: LP 完全跳过该约束.
- 某煤缺少约束所需指标: 自动剔除该煤, 并在 `BlendResult::warnings` 中记录.
- LP 不可行: `BlendResult::ok = false`, `reason` 说明原因.

## 运行

```bash
# 编译
cargo build --release

# 测试 (10 项, 含 binding 检测/容错/三视图/CSR 预测)
cargo test --release

# 8 煤完整 demo (含性能基准 + 三视图输出)
cargo run --release --example demo
```

## 性能基准

M1 Mac, release + LTO:

```
n=8 煤, 7 项合同约束:  0.113 ms/次
```

## 扩展点

可选 CSR 预测模块位于 `src/predict.rs`, 如需在调用 `solve` 前对 `Coal.props["CSR"]` 进行公式预计算可接入该模块. 当前主路径不依赖预测, 直接使用技术部门录入的经验值.
