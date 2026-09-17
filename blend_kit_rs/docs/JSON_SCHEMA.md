# 豆哥配煤 JSON API 契约

最后更新：2026-07-17。`blend_kit::solve_json` 是 WASM 与 HTTP 服务端共用的
JSON 字符串边界；字段使用 Rust `serde` 默认的 `snake_case`。

## 请求：`BlendRequest`

```json
{
  "coals": [{
    "name": "临北",
    "props": {"S": 2.0, "A": 6.0, "V": 22.0, "G": 93.0,
              "Y": 17.0, "petro": 0.08, "CSR": 70.0, "M": 11.0},
    "fob": 1425.0,
    "frt": 25.0,
    "petrography": {
      "hist": [[1.15, 10.0], [1.25, 20.0]],
      "vitrinite_pct": 80.0,
      "mean": 1.22,
      "std_dev": 0.05
    }
  }],
  "specs": [{
    "indicator": "S",
    "direction": "Upper",
    "max": 2.5,
    "acceptance": {"mode": "Truncate", "decimals": 1, "tolerance": 0.0},
    "enforcement": "Hard",
    "margin": 0.02
  }],
  "total_quantity": 3700.0,
  "truncate_decimal": true
}
```

必填字段是 `coals`、`specs`、煤名、`props`、`fob` 和 `frt`。煤名必须唯一，
数量须为正数，价格及指标须为非负有限数。

### 合同判定

- `direction`：`Upper`、`Lower` 或 `Range`。
- `acceptance.mode`：`Raw`、`Truncate` 或 `Round`；`decimals` 为 0–6。
- `tolerance` 是合同允许偏差；`margin` 是模型风险导致的内部收紧，两者不可混用。
- `enforcement`：`Hard` 进入 LP，`Soft` 不阻断但影响总体状态，
  `Advisory` 只展示。
- 未提供 `acceptance` 时，兼容 `truncate_decimal=true` 的旧一位小数截断规则；
  岩相不套用此旧规则。例如 S 上限 2.5 时，2.5999 可截断判定为 2.5，
  2.6000 不可通过。

### 可选模型复核

G 与 CSR 可以由调用方显式启用校正模型。模型只有通过最小样本数、留一交叉验证
MAE 和训练域门槛后才可参与 Hard 约束；域外结果只作为未验证估算展示。未提供模型
时，求解器继续使用现有线性代理并标记为估算。该能力只通过 Rust
`solve_with_evaluators` 入口启用；`solve_json` 不接收样本，也不会自动训练。

## 响应：`BlendResult`

`ok` 仅表示是否得到可下单配方；可信度看 `quality_status`：

- `Verified`：所有非 Advisory 合同项均已验证；
- `Estimated`：存在透明标记的代理值或未验证项；
- `NeedsReview`：Soft 项失败或结果需人工复核。

`indicator_check` 对每项给出：

- `proxy_value`：LP/旧算法代理值；
- `evaluated_value`：G 校准、CSR 回归或岩相复验值；
- `judged_value`：按合同截断/四舍五入后的判定值；
- `method`、`status`、`uncertainty` 与可选 `model` 摘要。

`status` 为 `Pass`、`TolerancePass`、`Unverified` 或 `Fail`。
岩相另在 `petrography_check` 返回混合均值、全方差 σ、凹口及修复轮数。
Hard 岩相精确复验失败时返回 `ok=false`。

旧请求可省略新增判定字段，继续使用原有一位小数截断行为。
