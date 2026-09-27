# 豆哥配煤环境

这个沙箱预装了 `blend`，它和线上豆哥配煤 App 用的是同一套配煤核心。
直接调用它，不要自己写代码重算指标。

```bash
blend master                  # 内置煤种库与默认合同 (JSON)
blend solve  < request.json   # 求最低成本配方
blend eval   < request.json   # 验算: 按给定配比出结果, 请求须带 fixed_ratios
```

输入、输出都是 JSON。退出码：0 = 有结果；1 = 求不出，原因在 `reason`；2 = 用法或 JSON 错误。

## 请求格式 (BlendRequest)

```json
{
  "coals": [
    {"name": "荣欣", "fob": 2400, "frt": 60,
     "props": {"S": 0.4, "A": 9.7, "V": 24, "G": 97, "Y": 23, "petro": 0.1, "CSR": 75, "M": 10}},
    {"name": "新星", "fob": 1550, "frt": 80,
     "props": {"S": 1.6, "A": 10.5, "V": 20, "G": 75, "Y": 17, "petro": 0.1, "CSR": 65, "M": 10}}
  ],
  "specs": [
    {"indicator": "S", "direction": "Upper", "max": 2.5},
    {"indicator": "G", "direction": "Lower", "min": 80}
  ],
  "total_quantity": null,
  "truncate_decimal": true,
  "fixed_ratios": {"荣欣": 2, "新星": 4}
}
```

- `props` 的 8 项指标：S 硫、A 灰、V 挥发 (Vdaf)、G 粘结、Y 胶质、petro 岩相、CSR 焦炭反应后强度、M 水分
- `specs` 可以直接用 `blend master` 输出里的 `default_contract.specs`
- `fixed_ratios` 是份数，按总和归一 (2:4:3:2 直接填)。只有 `eval` 用它，`solve` 不能带
- 合同里某项是硬约束、而煤缺这一项时，这种煤会被剔出煤池。数据确实没有这一项时，把那条 spec 的 `enabled` 设为 `false`

## 读结果 (BlendResult)

- `recipe`：煤名 → 配比
- `cost`：`cif_per_ton` 到厂价，`net_per_ton` 含扣款的实际成本
- `indicator_check[]`：每项的 `value` 和 `status`
  - `Pass` 通过；`Fail` 超标
  - `Unverified` 按线性加权估算，未经验证（挥发、G、Y、岩相、CSR 属于这类）
- `quality_status`：`Verified` / `Estimated` / `NeedsReview`（有超标项）
- `warnings`：被剔除的煤及原因
