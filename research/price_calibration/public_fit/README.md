# 公开报价标定 (2026-11-04)

用公开数据标定估价系数：取公开的山西出厂含税报价，每条只用当天钢铁网 7 条分煤种指数去估。
结论和数字写在 master `price_factor.matching_note` 与 `description` 里。

数据不进仓库：仓库是公开的，指数和报价是钢铁网、中焦协的数据。需要的两份数据在本地生成：

```bash
# 一年指数 (线上服务已入库)
cd blend_kit_server && cargo run -q --release --bin doudou -- --profile admin api GET /api/mcci \
  > ../research/price_calibration/public_fit/mcci.json
# 公开报价 (本地 research/price_calibration/samples_comparable.csv, 来源与口径见同目录 evidence_*.md)
cd ../research/price_calibration/public_fit && node to_json.mjs ../samples_comparable.csv > public.json
```

脚本：

- `fit.mjs`：筛样本（山西、报价、不含承兑、指标为单值且带 V），按一套系数估价，按产品组等权计分。
- `grid2.mjs`：网格搜索；要求快讯、周报两组来源各自的误差都低。
- `cmp.mjs`：按产品组比较几套系数。

```bash
cd blend_kit_rs && cargo build --release --bin blend && cd ../research/price_calibration/public_fit
node grid2.mjs public.json mcci.json ../../../blend_kit_rs/data/coal_master.json ../../../blend_kit_rs/target/release/blend
```
