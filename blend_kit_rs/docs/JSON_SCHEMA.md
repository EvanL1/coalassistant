# 豆哥配煤 JSON API 契约

最后更新：2026-11-17。`blend_kit::solve_json` 是命令行与 HTTP 服务端共用的
JSON 字符串边界；字段使用 Rust `serde` 默认的 `snake_case`。

## 请求：`BlendRequest`

```json
{
  "coals": [{
    "name": "临北",
    "props": {"S": 2.0, "A": 6.0, "V": 22.0, "G": 93.0,
              "Y": 17.0, "petro": 0.08, "CSR": 70.0, "M": 11.0},
    "fob": 1425.0,
    "frt": 25.0
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
数量须为正数，价格及指标须为非负有限数。不认识的字段一律忽略 —— 包括旧请求里的
煤岩明细 `petrography`（反射率直方图 / μ、σ）：它已不是输入，岩相只按 `props.petro`
这个标量按配比加权。

### 合同判定

- `direction`：`Upper`、`Lower` 或 `Range`。
- `acceptance.mode`：`Raw`、`Truncate` 或 `Round`；`decimals` 为 0–6。
- `tolerance` 是合同允许偏差；`margin` 是内部留的安全余量（只收紧求解），两者不可混用。
- `enforcement`：`Hard` 进入 LP，`Soft` 不阻断但影响总体状态，
  `Advisory` 只展示，`Priced` 超合同界按 `penalty` 扣款、越过拒收线 `penalty.reject`
  才不可行。拒收线与合同界按同一条判定规则判：判定值越过它才算越线。
- 未提供 `acceptance` 时，兼容 `truncate_decimal=true` 的旧一位小数截断规则；
  岩相不套用此旧规则。例如 S 上限 2.5 时，2.5999 可截断判定为 2.5，
  2.6000 不可通过。

### 八项指标都是线性加权

八项指标（含 G、CSR、岩相）都按配比线性加权。硫、灰、水分可精确复算，标 `Linear`；
其余是线性估算，标 `ProvisionalLinear`，状态为 `Unverified`。求解不接收训练样本，
也没有校正模型；实测 CSR 的回归只在后台看样本进度，不进配方。

## 响应：`BlendResult`

`ok` 仅表示是否得到可下单配方；可信度看 `quality_status`：

- `Verified`：所有非 Advisory 合同项均已验证；
- `Estimated`：存在透明标记的代理值或未验证项；
- `NeedsReview`：Soft 项失败或结果需人工复核。

`indicator_check` 对每项给出：

- `value`：按配比加权的混合值；
- `judged_value`：按合同截断/四舍五入后的判定值；
- `method`：`Linear`、`ProvisionalLinear` 或 `Unavailable`（算不出）；`status` 见下；
- `proxy_value`、`evaluated_value`：与 `value` 是同一个量，为兼容保留；
- `uncertainty`、`model`：恒为 `null`，为兼容保留。

`status` 为 `Pass`、`TolerancePass`、`Unverified` 或 `Fail`。
`evaluation_iterations` 恒为 0，为兼容保留。以前存下的结果里可能还有
`petrography_check` 与 `Histogram` / `Moments` / `AffineCalibration` / `Regression`
这几种 `method`，现在不会再产出。

旧请求可省略新增判定字段，继续使用原有一位小数截断行为。
