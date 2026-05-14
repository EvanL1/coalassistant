# 豆哥配煤 JSON API 契约

**最后更新:** 2026-05-14  
**适用对象:** Flutter / Tauri 前端开发者  
**Ground truth:** `src/model.rs` + `src/optimizer.rs`

---

## 1. 概览

`solve_json(input_json: &str) -> String` 是 Rust 核心对外暴露的唯一接口。

- **输入:** 一个 UTF-8 JSON 字符串，结构为 `BlendRequest`
- **输出:** 一个 UTF-8 JSON 字符串，结构为 `BlendResult`
- **功能:** 给定煤池（化验值 + 价格）和合同约束，用线性规划求最低成本配方，输出三个业务视图（成本结构 / 实物订单 / 指标体检）
- **字段命名规则:** 全部 `snake_case`（serde 默认，未设置 `rename_all`）

---

## 2. 请求 Schema — BlendRequest

### 顶层字段

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `coals` | `Coal[]` | 是 | — | 煤池，至少 1 条 |
| `specs` | `Spec[]` | 是 | — | 合同约束，可为空数组 |
| `total_quantity` | `number \| null` | 否 | `null` | 总采购吨数；为 null 时只输出比例，不算实物订单金额 |
| `truncate_decimal` | `boolean` | 否 | `true` | 是否启用一位小数截断规则（见第 4 节） |

### 2.1 Coal 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | `string` | 是 | 煤种名称，用于输出标识 |
| `props` | `object` | 是 | 8 项指标的键值对，值为 `number`（浮点）；**缺失项不填**（会触发容错剔除，见第 5 节） |
| `fob` | `number` | 是 | 出厂价，元/吨 |
| `frt` | `number` | 是 | 运费，元/吨 |

#### props 中的 8 项指标键

| key | 中文 | 单位 | 性质 | 小焦炉典型范围 |
|-----|------|------|------|----------------|
| `S` | 硫 | % | 越低越好 | 1.0 ~ 4.0 |
| `A` | 灰 | % | 越低越好 | 5.0 ~ 12.0 |
| `V` | 挥发 | % | 目标范围 | 16.0 ~ 28.0 |
| `G` | 粘结 | 无量纲 | 越高越好 | 60 ~ 100 |
| `Y` | 胶质 | mm | 越高越好 | 8 ~ 25 |
| `petro` | 岩相（最大反射率） | 无量纲 | 目标范围 | 0.05 ~ 0.25 |
| `CSR` | 焦炭强度 | 无量纲 | 越高越好 | 50 ~ 70 |
| `M` | 水分 | % | 越低越好 | 6.0 ~ 12.0 |

> **注意:** `props` 可以只填部分指标。若某煤缺少某条 `Spec` 所对应的指标，该煤会被自动剔除并在 `warnings` 中记录（详见第 5 节）。

### 2.2 Spec 字段

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `indicator` | `string` | 是 | — | 8 项 key 之一（`"S"` / `"A"` / `"V"` / `"G"` / `"Y"` / `"petro"` / `"CSR"` / `"M"`） |
| `direction` | `"Upper" \| "Lower" \| "Range"` | 是 | — | 约束方向（枚举值大写开头） |
| `min` | `number \| null` | 条件 | `null` | `Lower` / `Range` 时须填 |
| `max` | `number \| null` | 条件 | `null` | `Upper` / `Range` 时须填 |
| `enabled` | `boolean` | 否 | `true` | `false` 时该约束完全跳过，不参与 LP |

#### Direction 枚举含义

| 值 | 含义 | 生效字段 | 典型指标 |
|----|------|----------|----------|
| `"Upper"` | 不超过上限（越低越好） | `max` | 硫、灰、水分 |
| `"Lower"` | 不低于下限（越高越好） | `min` | 粘结、胶质、焦炭强度 |
| `"Range"` | 限定范围 | `min` + `max` | 挥发、岩相 |

### 2.3 完整请求示例

```json
{
  "coals": [
    {
      "name": "铁新",
      "props": { "S": 3.2, "A": 6.5, "V": 21.0, "G": 92.0, "Y": 14.0, "petro": 0.10, "CSR": 62.0, "M": 7.5 },
      "fob": 1260.0,
      "frt": 30.0
    },
    {
      "name": "临北",
      "props": { "S": 2.0, "A": 6.0, "V": 22.0, "G": 93.0, "Y": 16.0, "petro": 0.10, "CSR": 66.0, "M": 7.0 },
      "fob": 1250.0,
      "frt": 30.0
    },
    {
      "name": "安益",
      "props": { "S": 3.4, "A": 6.8, "V": 18.0, "G": 75.0, "Y": 10.0, "petro": 0.18, "CSR": 60.0, "M": 8.0 },
      "fob": 1120.0,
      "frt": 30.0
    },
    {
      "name": "筛精",
      "props": { "S": 3.9, "A": 9.5, "V": 25.0, "G": 100.0, "Y": 22.0, "petro": 0.08, "CSR": 65.0, "M": 9.5 },
      "fob": 970.0,
      "frt": 30.0
    },
    {
      "name": "孟子峪",
      "props": { "S": 2.0, "A": 9.5, "V": 18.0, "G": 75.0, "Y": 10.0, "petro": 0.18, "CSR": 58.0, "M": 7.8 },
      "fob": 1034.0,
      "frt": 30.0
    },
    {
      "name": "大佛寺",
      "props": { "S": 3.0, "A": 8.5, "V": 18.0, "G": 75.0, "Y": 10.0, "petro": 0.16, "CSR": 59.0, "M": 8.0 },
      "fob": 1120.0,
      "frt": 30.0
    },
    {
      "name": "神州",
      "props": { "S": 2.6, "A": 10.0, "V": 17.0, "G": 65.0, "Y": 8.0, "petro": 0.20, "CSR": 55.0, "M": 7.2 },
      "fob": 1110.0,
      "frt": 30.0
    },
    {
      "name": "豹子沟",
      "props": { "S": 3.8, "A": 11.0, "V": 24.0, "G": 92.0, "Y": 20.0, "petro": 0.12, "CSR": 64.0, "M": 8.5 },
      "fob": 1250.0,
      "frt": 30.0
    }
  ],
  "specs": [
    { "indicator": "S", "direction": "Upper", "max": 2.5 },
    { "indicator": "A", "direction": "Upper", "max": 9.0 },
    { "indicator": "V", "direction": "Range",  "min": 18.0, "max": 27.0 },
    { "indicator": "G", "direction": "Lower",  "min": 80.0 },
    { "indicator": "Y", "direction": "Lower",  "min": 14.0 },
    { "indicator": "M", "direction": "Upper",  "max": 12.0 },
    { "indicator": "CSR", "direction": "Lower", "min": 60.0 }
  ],
  "total_quantity": 3700.0,
  "truncate_decimal": true
}
```

---

## 3. 响应 Schema — BlendResult

### 顶层字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `ok` | `boolean` | `true` = 求解成功；`false` = 失败 |
| `reason` | `string \| null` | 失败原因；成功时为 `null` |
| `recipe` | `object` | 煤名 → 配比（0~1），仅含配比 > 0.00001 的煤 |
| `cost` | `CostBreakdown \| null` | 视图 A：成本结构；失败时为 `null` |
| `orders` | `OrderItem[]` | 视图 B：实物订单，按配比降序；失败时为空数组 |
| `indicator_check` | `IndicatorCheck[]` | 视图 C：指标体检，按 INDICATORS 顺序；失败时为空数组 |
| `warnings` | `string[]` | 容错警告（煤被剔除等），成功时也可能非空 |

### 3.1 CostBreakdown 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `fob_per_ton` | `number` | 加权出厂价，元/吨 |
| `frt_per_ton` | `number` | 加权运费，元/吨 |
| `cif_per_ton` | `number` | 到厂综合价（= fob + frt），元/吨 |
| `total_fob` | `number \| null` | 总出厂金额；仅当请求含 `total_quantity` 时填充 |
| `total_frt` | `number \| null` | 总运费金额；仅当请求含 `total_quantity` 时填充 |
| `total_cif` | `number \| null` | 总到厂金额；仅当请求含 `total_quantity` 时填充 |

### 3.2 OrderItem 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `coal` | `string` | 煤种名称 |
| `ratio` | `number` | LP 求解配比（0~1）。所有 orders 之和 ≈ 1（过滤掉了 ratio < 1e-5 的煤） |
| `tons` | `number \| null` | 采购吨数 = `ratio × total_quantity`；无 `total_quantity` 时为 `null` |
| `fob_amount` | `number \| null` | 出厂金额 = `tons × fob`；无 `total_quantity` 时为 `null` |
| `frt_amount` | `number \| null` | 运费金额 = `tons × frt`；无 `total_quantity` 时为 `null` |
| `cif_amount` | `number \| null` | 到厂金额 = `tons × cif`；无 `total_quantity` 时为 `null` |

### 3.3 IndicatorCheck 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `indicator` | `string` | 指标 key（`"S"` / `"A"` 等） |
| `label_zh` | `string` | 中文标签（`"硫"` / `"灰"` 等） |
| `value` | `number` | 混合后的加权实际值 |
| `min` | `number \| null` | 该指标的下限约束（若无则为 `null`） |
| `max` | `number \| null` | 该指标的上限约束（若无则为 `null`） |
| `slack` | `number \| null` | 距最近边界的余量。`null` = 无约束，正值 = 未顶格，负值 = 已违反 |
| `binding` | `boolean` | `slack` 接近 0 且非负（卡到合同边界）时为 `true`，是谈判方向的归因依据 |

> **注意:** `indicator_check` 仅包含煤池数据完整的指标（所有保留煤均有该指标的值）。若某指标在部分煤中缺失，该指标不出现在 `indicator_check` 里。

### 3.4 成功响应示例

```json
{
  "ok": true,
  "reason": null,
  "recipe": {
    "临北": 0.4521,
    "筛精": 0.3102,
    "铁新": 0.2377
  },
  "cost": {
    "fob_per_ton": 1181.34,
    "frt_per_ton": 30.0,
    "cif_per_ton": 1211.34,
    "total_fob": 4370957.8,
    "total_frt": 111000.0,
    "total_cif": 4481957.8
  },
  "orders": [
    {
      "coal": "临北",
      "ratio": 0.4521,
      "tons": 1672.77,
      "fob_amount": 2091962.5,
      "frt_amount": 50183.1,
      "cif_amount": 2142145.6
    },
    {
      "coal": "筛精",
      "ratio": 0.3102,
      "tons": 1147.74,
      "fob_amount": 1113307.8,
      "frt_amount": 34432.2,
      "cif_amount": 1147740.0
    },
    {
      "coal": "铁新",
      "ratio": 0.2377,
      "tons": 879.49,
      "fob_amount": 1108557.0,
      "frt_amount": 26384.7,
      "cif_amount": 1134941.7
    }
  ],
  "indicator_check": [
    {
      "indicator": "S",
      "label_zh": "硫",
      "value": 2.5,
      "min": null,
      "max": 2.5,
      "slack": 0.0999,
      "binding": true
    },
    {
      "indicator": "A",
      "label_zh": "灰",
      "value": 7.81,
      "min": null,
      "max": 9.0,
      "slack": 1.29,
      "binding": false
    },
    {
      "indicator": "V",
      "label_zh": "挥发",
      "value": 22.6,
      "min": 18.0,
      "max": 27.0,
      "slack": 4.4,
      "binding": false
    },
    {
      "indicator": "G",
      "label_zh": "粘结",
      "value": 94.8,
      "min": 80.0,
      "max": null,
      "slack": 14.8,
      "binding": false
    },
    {
      "indicator": "Y",
      "label_zh": "胶质",
      "value": 18.1,
      "min": 14.0,
      "max": null,
      "slack": 4.1,
      "binding": false
    },
    {
      "indicator": "petro",
      "label_zh": "岩相",
      "value": 0.093,
      "min": null,
      "max": null,
      "slack": null,
      "binding": false
    },
    {
      "indicator": "CSR",
      "label_zh": "焦炭强度",
      "value": 64.2,
      "min": 60.0,
      "max": null,
      "slack": 4.2,
      "binding": false
    },
    {
      "indicator": "M",
      "label_zh": "水分",
      "value": 7.9,
      "min": null,
      "max": 12.0,
      "slack": 4.1,
      "binding": false
    }
  ],
  "warnings": []
}
```

> 上述 JSON 中 `slack` 为 `null` 的情况：当该指标无约束（`min` 和 `max` 均为 `null`）时，Rust 内部使用 `f64::INFINITY`，序列化为 JSON 时大多数环境会输出 `null` 或特殊值。前端应将 `null` / 缺失的 `slack` 视为"无约束"处理。

### 3.5 失败响应示例（约束冲突）

```json
{
  "ok": false,
  "reason": "约束冲突, LP 不可行",
  "recipe": {},
  "cost": null,
  "orders": [],
  "indicator_check": [],
  "warnings": [
    "剔除 神州: 缺指标 胶质"
  ]
}
```

---

## 4. 截断规则说明（truncate_decimal）

`truncate_decimal: true`（默认）时，所有上限约束（`Upper` / `Range` 的 max）在 LP 内部会放宽 **0.0999**：

```
LP 实际上限 = max + 0.0999
```

**目的:** 避免一位小数的化验数据因浮点误差略超限而判不可行。

**示例:** 合同要求硫 ≤ 2.5%：

| truncate_decimal | LP 内部上限 | `indicator_check.max` 显示 | `indicator_check.slack` 含义 |
|-----------------|-------------|---------------------------|------------------------------|
| `true`（默认）  | 2.5999      | 2.5（原始合同值）          | 距放宽后上限的余量 |
| `false`         | 2.5         | 2.5                        | 距原始上限的余量 |

**注意:** `indicator_check.max` 始终显示原始合同值（2.5），不显示放宽后的值。`slack` 是距放宽后有效上限（2.5999）的余量，因此 `binding` 的判定也基于放宽后的上限。

**下限约束不受影响:** `Lower` 方向的 `min` 不做任何放宽。

---

## 5. 容错行为

| 场景 | 触发条件 | 处理方式 | 对结果的影响 |
|------|----------|----------|--------------|
| 煤缺关键指标 | 某煤的 `props` 缺少某条 `enabled=true` 的 Spec 所对应的 key | 该煤从煤池中剔除，加一条 `warnings` | `recipe` 和 `orders` 不含该煤 |
| Spec 被禁用 | `Spec.enabled = false` | 完全跳过，不加入 LP 约束 | 该指标不影响配方，但若煤池数据全，仍出现在 `indicator_check` 中（无 min/max） |
| 煤池全被剔除 | 所有煤都缺指标 | 返回 `ok=false`，`reason="无可用煤"` | 失败响应 |
| LP 不可行 | 约束矛盾（如同时要求高硫又要低硫） | 返回 `ok=false`，`reason="约束冲突, LP 不可行"` | 失败响应，`warnings` 保留剔除信息 |
| 指标在部分煤中缺失 | 煤池中有煤不含某指标（即使该指标无 Spec） | 该指标跳过，不出现在 `indicator_check` | 仅影响体检视图完整性 |
| JSON 格式错误 | `input_json` 无法解析 | 返回 `ok=false`，`reason="JSON 解析失败: ..."` | 失败响应 |

---

## 6. 常见错误响应

### JSON 解析失败

```json
{
  "ok": false,
  "reason": "JSON 解析失败: missing field `coals` at line 1 column 2",
  "recipe": {},
  "cost": null,
  "orders": [],
  "indicator_check": [],
  "warnings": []
}
```

### 无可用煤（全部被剔除）

```json
{
  "ok": false,
  "reason": "无可用煤",
  "recipe": {},
  "cost": null,
  "orders": [],
  "indicator_check": [],
  "warnings": [
    "剔除 临北: 缺指标 胶质",
    "剔除 铁新: 缺指标 胶质/粘结"
  ]
}
```

### 约束冲突（LP 不可行）

```json
{
  "ok": false,
  "reason": "约束冲突, LP 不可行",
  "recipe": {},
  "cost": null,
  "orders": [],
  "indicator_check": [],
  "warnings": []
}
```

---

## 7. 不提供 total_quantity 时的响应差异

当请求中 `total_quantity` 为 `null` 或省略时：

- `cost.total_fob` / `total_frt` / `total_cif` 均为 `null`
- `orders[i].tons` / `fob_amount` / `frt_amount` / `cif_amount` 均为 `null`
- `recipe` 和 `orders[i].ratio` 正常输出

适用场景：只需要配比和单位成本，不需要具体采购量时使用。

---

## 8. 小焦炉表单校验参考范围

前端可用以下范围做输入校验（非 LP 约束，仅用于 UI 提示）：

| 指标 | 输入最小值 | 输入最大值 | 建议 Spec 范围示例 |
|------|-----------|-----------|-------------------|
| S（硫） | 0.1 | 6.0 | Upper max ≤ 3.0 |
| A（灰） | 3.0 | 15.0 | Upper max ≤ 10.0 |
| V（挥发） | 12.0 | 32.0 | Range 18.0 ~ 28.0 |
| G（粘结） | 30 | 100 | Lower min ≥ 65 |
| Y（胶质） | 0 | 30 | Lower min ≥ 10 |
| petro（岩相） | 0.01 | 0.35 | Range 0.05 ~ 0.25 |
| CSR（焦炭强度） | 40 | 75 | Lower min ≥ 58 |
| M（水分） | 3.0 | 14.0 | Upper max ≤ 12.0 |
