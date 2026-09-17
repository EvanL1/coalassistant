# 煤库数据更新接口

给自动化（脚本 / AI）往煤库补指标用的写入接口。人工看的煤池编辑在前端，不走这里。

## 数据分两层

```
GET /api/master
    基线  blend_kit_rs/data/coal_master.json   编译期嵌入, 进 git, 可 diff 可 review
  + 覆盖层 PostgreSQL coal_overrides 表         本接口写入, 立即生效, 不用重新部署
  = 返回给前端
```

**基线永远优先。** 覆盖层只补基线缺的指标，补上的值会写进该煤的 `props` /
`confidence`，并在 `note` 上追加 `[覆盖层] Y=16 (来源...)` 留痕。

因此：

- 删掉覆盖层一行 = 回滚一个值
- 清空 `coal_overrides` 表 = 回到 git 里那份基线
- 覆盖层坏了 / 数据库连不上 → 自动退回纯基线，煤库不会整个不可用

## 鉴权

所有接口要求请求头 `X-API-Key`。服务端在 `DATA_API_KEYS` 里配了一组**具名**密钥
（格式 `名字:密钥,名字:密钥`），**一个持有者一把**。

- 未配置任何密钥 → 整组接口返回 **503**，不是放行
- 密钥错或缺失 → **401**
- 用户登录的会话 Cookie **不能**替代这把密钥，反之亦然

密钥对应的名字会作为 `updated_by` 落库，**由服务端推导，调用方伪造不了**——请求体里
塞 `updated_by` 不起作用。成功响应会回显你的身份：

```json
{"ok": true, "applied": 2, "updated_by": "friend"}
```

撤销某个持有者只需从 `DATA_API_KEYS` 里删掉他那一段并重启，其他人不受影响。

密钥检查发生在**请求体被解析之前**：畸形 JSON、类型错误、缺 `Content-Type` 的未鉴权请求
一律只拿到 401/503，不会回显字段名或类型错误。（这点有测试钉着：
`test_malformed_body_does_not_bypass_auth`。）

## POST /api/master/coals

批量补充指标。

```bash
curl -X POST https://<host>/api/master/coals \
  -H "X-API-Key: $MY_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "updates": [
      { "coal": "铁新", "field": "Y",   "value": 16,
        "source": "2026-09-17 Mysteel 规格 Y≥16",   "confidence": "medium" },
      { "coal": "铁新", "field": "CSR", "value": 60,
        "source": "2026-09-17 Mysteel 规格 CSR≥60", "confidence": "medium" }
    ]
  }'
```

字段说明：

| 字段 | 必填 | 说明 |
|------|------|------|
| `coal` | 是 | 煤名，**必须已存在于基线**。本接口不新增煤源 |
| `field` | 是 | `S` `A` `V` `G` `Y` `petro` `CSR` `M` 之一 |
| `value` | 是 | 数值，须落在该指标物理量程内（见下） |
| `source` | 是 | 来源与口径，不能为空，≤ 200 字符。**保留原始 `≥`/`≤` 符号** |
| `confidence` | 否 | `high` / `medium` / `low`，默认 `medium` |

`updated_by` 不用填也填不了——服务端从你的密钥推导。

限额：单批 **≤ 300 条**，请求体 **≤ 256KB**。合法批次天然封顶——基线 112 座煤 × 8 项
减去已填的，最多两百余条空缺，所以 300 不会误伤正常用法。`source` 封顶是因为它会写进
`note`，而 `note` 每次 `GET /api/master` 都要回给前端，不封顶等于让主数据请求被永久撑大。

成功：`{"ok": true, "applied": 2, "updated_by": "friend"}`

## 三条必须知道的规则

**① 只填空，不覆盖。** 基线已有的指标一律拒绝（422）：

```json
{"ok": false, "errors": [
  "第 1 条 (临北/S): 基线已有该指标 (=2), 本接口只补空缺; 要修正已有值请改 coal_master.json 并提 PR"
]}
```

这不是麻烦，是有意的。基线里的值多是人工核过的实测化验单，而自动化推来的多是
Mysteel 规格界限（`≥`/`≤`）。让界限值盖过实测值，求解器会以为配方合规而实际不合规。
要改已有值 → 改 `coal_master.json` 提 PR，走人工审阅。

**② 整批校验，一条不合法则全批不写。** 半批生效会让调用方分不清实际状态。

**③ 数值必须落在物理量程内：**

| 指标 | 量程 | | 指标 | 量程 |
|------|------|---|------|------|
| S 硫 | 0–10 | | Y 胶质 | 0–50 |
| A 灰 | 0–50 | | petro 岩相 | **0–1** |
| V 挥发 | 0–50 | | CSR | 0–100 |
| G 粘结 | 0–100 | | M 水分 | 0–30 |

岩相尤其要注意：它是反射率分布度量，现有数据都在 0.07–0.13。2026-09-17 的一份调研表
曾把年份 `2026` 填进这一列，就是靠这道量程拦下的。

同一套量程在三处都有：应用层校验、数据库 CHECK 约束、`scripts/check_master_data.mjs`。

## GET /api/master/overrides

看当前覆盖层里有什么——排查"这个值到底哪来的"。

```bash
curl https://<host>/api/master/overrides -H "X-API-Key: $MY_API_KEY"
```

## DELETE /api/master/coals/{coal}/{field}

回滚一个值到基线状态。

```bash
curl -X DELETE "https://<host>/api/master/coals/铁新/CSR" \
  -H "X-API-Key: $MY_API_KEY"
```

覆盖层里没有这一条时返回 404。

## 状态码

| 码 | 含义 |
|----|------|
| 200 | 成功 |
| 400 | 请求体不是合法 JSON，或超出 256KB（**鉴权通过后**才会看到这个码）|
| 401 | `X-API-Key` 缺失或错误 |
| 404 | DELETE 的目标不在覆盖层里 |
| 422 | 数据不合法，`errors` 数组逐条说明。整批未写入 |
| 503 | 未配置 `DATA_API_KEYS`，或数据库不可用 |

## 什么该走这里，什么不该

**适合：** 调研表出了新版，补上基线缺的 Y/CSR/M 之类指标。

**不适合，应走 git PR：**

- 修正基线已有的值（接口会拒）
- 新增煤源（接口会拒，煤名必须已在基线里）
- 大批量结构性变更

判断依据很简单：**这个改动需要人看一眼吗？** 需要就走 git —— 那里有 diff、有 review、
有历史。覆盖层是为"机械补空缺"准备的，不是绕过审阅的后门。
