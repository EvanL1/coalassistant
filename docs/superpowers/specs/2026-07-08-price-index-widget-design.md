# 焦煤指数窄条（今日屏）设计

日期: 2026-07-08
状态: 已批准（用户确认放置位置与设计）

## 目的

在今日屏顶部加一条焦煤行情窄条：当前价 + 涨跌幅 + 近 30 日迷你曲线，让配煤师做配比决策时顺眼看到市场水平。本期只做展示——为后续"价格快照 + 指数漂移"铺垫，但不接入任何求解或价格逻辑。

## 数据源（2026-07-08 实测验证）

- **主用：东方财富日 K 接口**

  ```
  https://push2his.eastmoney.com/api/qt/stock/kline/get
    ?secid=114.jmm&klt=101&fqt=0
    &fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57
    &end=20500101&lmt=60
  ```

  - `secid=114.jmm` = 大商所焦煤主力连续（响应 name 为"焦煤主连"）
  - 响应带 `Access-Control-Allow-Origin` 回显任意 Origin → 浏览器与 Tauri webview 均可直接 `fetch`，无需后端
  - `klines` 每条形如 `"2013-03-25,1280.0,1267.0,1304.0,1257.0,422756,0.0"`（日期,开,收,高,低,量,…；收盘价取第 3 个字段，实现时以实际响应核对）
  - 注意 `lmt` 参数在带 `beg=0` 时未按预期截尾，实现时省略 `beg` 或取返回数组末尾 30 条

- **备胎：新浪日 K JSONP** `stock2.finance.sina.com.cn/futures/api/jsonp.php/.../InnerFuturesNewService.getDailyKLine?symbol=JM0`（无 Referer 可用，但返回 2013 年至今全量且是 JSONP，仅当东财失效再考虑）
- **已排除：`hq.sinajs.cn`**（无新浪域 Referer 即 Forbidden，前端场景不可用）

## 组件

### `src/index_quote.ts`（数据层，纯 TS，无 React）

- `fetchJmKline(): Promise<JmKline | null>`，`JmKline = { points: { date: string; close: number }[], fetchedAt: string, stale: boolean }`
- localStorage 缓存 key `doudou_jm_kline`；缓存 10 分钟内直接返回不发请求（今日屏每次切 tab 重挂载，避免反复打接口）
- fetch 失败 → 返回缓存并 `stale: true`；无缓存 → 返回 `null`
- 响应解析独立成纯函数（无 IO），为将来补测留口

### `src/IndexTicker.tsx`（展示层）

- 单行窄条：`焦煤JM主力  1293.5  ▲+0.4%  [sparkline]`
- 涨跌幅 = 最后一根收盘 vs 前一根收盘；红涨绿跌（国内习惯）
- sparkline：内联 SVG `<polyline>`，近 30 点，不引图表库
- `stale` 时价格旁灰字标最后数据日期（MM-DD）
- 无数据（离线且无缓存）→ 返回 `null`，整条不渲染
- 挂载点：`TodayScreen` 顶部第一行

## 不做（YAGNI）

- 点击交互 / 展开大图
- 多品种、多周期切换
- 指数值写入 mines / 求解 / 漂移公式（属于下一期"价格快照 + 指数锚定"）
- 任何 Rust / backend seam 改动，任何新 npm 依赖

## 验证

- `npm run dev` 浏览器端：正常显示；断网刷新 → 显示缓存值 + 日期灰字；清缓存断网 → 窄条消失且无报错
- `npm run tauri dev` 原生端：同样三态
- 前端暂无测试框架，本期人工验证双端；解析纯函数保持可测形态
