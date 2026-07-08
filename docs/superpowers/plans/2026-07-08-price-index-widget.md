# 焦煤指数窄条 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 今日屏顶部加一条焦煤 JM 主力窄条：当前价 + 涨跌幅 + 近 30 日 SVG sparkline，数据来自东方财富公开日 K 接口（已实测 CORS 开放）。

**Architecture:** 纯前端两个新文件——`index_quote.ts`（fetch + localStorage 10 分钟缓存 + 纯函数解析，失败退回缓存标 stale）和 `IndexTicker.tsx`（单行展示 + 内联 SVG polyline），在 `App.tsx` 的 today 分支挂载（TodayScreen 有 4 个 return 分支，挂在 App 层只改一处且任何求解状态下都可见）。零 Rust 改动、零新依赖。

**Tech Stack:** React 19 + TypeScript + Vite；无测试框架（本仓库前端现状），解析用 Node 22 `--experimental-strip-types` 冒烟验证。

**Spec:** `docs/superpowers/specs/2026-07-08-price-index-widget-design.md`

**约定对齐（写代码前读这段）：**
- localStorage key 格式仿 `storage.ts`：`doudou_blend.<name>.v1`
- 颜色用 App.css 现有 CSS 变量：红涨 `var(--c-danger)`、绿跌 `var(--c-success)`、正文 `var(--c-text)`、弱化 `var(--c-text-3)`
- 注释与 UI 文案用中文
- commit 格式 `<type>(<scope>): <中文描述>`（hook 校验）

---

### Task 0: 前置检查（WASM pkg 存在，否则 tsc 都过不了）

**Files:** 无改动

- [ ] **Step 0.1: 检查 wasm pkg**

Run: `ls /Users/lyf/dev/coalassistant/blend_kit_wasm/pkg/package.json`

存在 → 跳到 Task 1。不存在 → `cd /Users/lyf/dev/coalassistant/blend_kit_wasm && wasm-pack build --target web --out-dir pkg --release`（首次约 1-2 分钟）。

---

### Task 1: 数据层 `index_quote.ts`

**Files:**
- Create: `doudou_blend/src/index_quote.ts`

- [ ] **Step 1.1: 写数据层模块（完整内容如下）**

```ts
/**
 * 焦煤指数行情 (大商所 JM 主力连续) — 纯前端数据层.
 *
 * 数据源: 东方财富日K接口, 响应带 CORS 头, 浏览器 / Tauri webview 均可直接 fetch.
 * 缓存: localStorage, 10 分钟内不重复请求; fetch 失败退回缓存并标 stale.
 * 设计: docs/superpowers/specs/2026-07-08-price-index-widget-design.md
 */

export interface KlinePoint {
  date: string; // "2026-07-08"
  close: number; // 收盘价 元/吨
}

export interface JmKline {
  points: KlinePoint[];
  fetchedAt: string; // 抓取时刻 ISO
  stale: boolean; // true = 本次抓取失败, 展示的是缓存旧值
}

const KEY_KLINE = "doudou_blend.jm_kline.v1";
const TTL_MS = 10 * 60 * 1000;
const POINTS = 30;

const API_URL =
  "https://push2his.eastmoney.com/api/qt/stock/kline/get" +
  "?secid=114.jmm&klt=101&fqt=0" +
  "&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57" +
  "&end=20500101&lmt=60";

/** 解析东财响应 (纯函数, 无 IO). klines 每条 "日期,开,收,高,低,量,额". */
export function parseEastmoneyKlines(json: unknown, take: number): KlinePoint[] {
  const klines = (json as { data?: { klines?: unknown } })?.data?.klines;
  if (!Array.isArray(klines)) return [];
  const points: KlinePoint[] = [];
  for (const line of klines) {
    if (typeof line !== "string") continue;
    const parts = line.split(",");
    const close = Number(parts[2]);
    if (parts[0] && Number.isFinite(close)) {
      points.push({ date: parts[0], close });
    }
  }
  return points.slice(-take);
}

function readCache(): JmKline | null {
  try {
    const raw = localStorage.getItem(KEY_KLINE);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as JmKline;
    if (!Array.isArray(parsed.points) || parsed.points.length === 0) return null;
    return parsed;
  } catch {
    return null;
  }
}

/** 取近 30 日焦煤主连日K. 缓存新鲜直接返回; 失败退缓存(stale); 全无 → null. */
export async function fetchJmKline(): Promise<JmKline | null> {
  const cached = readCache();
  if (cached && Date.now() - Date.parse(cached.fetchedAt) < TTL_MS) {
    return cached;
  }
  try {
    const resp = await fetch(API_URL);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const points = parseEastmoneyKlines(await resp.json(), POINTS);
    if (points.length === 0) throw new Error("empty klines");
    const fresh: JmKline = {
      points,
      fetchedAt: new Date().toISOString(),
      stale: false,
    };
    try {
      localStorage.setItem(KEY_KLINE, JSON.stringify(fresh));
    } catch {
      // 存不进缓存不影响本次展示
    }
    return fresh;
  } catch {
    return cached ? { ...cached, stale: true } : null;
  }
}
```

- [ ] **Step 1.2: Node 冒烟——纯函数解析 + 真实接口连通**

写临时脚本（放 scratchpad，不提交）`smoke_index_quote.mts`：

```ts
import { parseEastmoneyKlines } from "/Users/lyf/dev/coalassistant/doudou_blend/src/index_quote.ts";

// 1. 纯函数: 固定样例
const fixture = {
  data: { klines: ["2026-07-07,1280.0,1289.0,1304.0,1257.0,422756,0.0", "2026-07-08,1290.0,1293.5,1300.0,1285.0,400000,0.0"] },
};
const pts = parseEastmoneyKlines(fixture, 30);
console.assert(pts.length === 2 && pts[1].close === 1293.5 && pts[1].date === "2026-07-08", "解析失败", pts);

// 2. 畸形输入不炸
console.assert(parseEastmoneyKlines(null, 30).length === 0, "null 输入应返回空");
console.assert(parseEastmoneyKlines({ data: { klines: [42, "bad"] } }, 30).length === 0, "垃圾行应跳过");

// 3. 真实接口
const resp = await fetch(
  "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid=114.jmm&klt=101&fqt=0&fields1=f1,f2,f3,f4,f5&fields2=f51,f52,f53,f54,f55,f56,f57&end=20500101&lmt=60",
);
const live = parseEastmoneyKlines(await resp.json(), 30);
console.log("live points:", live.length, "last:", live[live.length - 1]);
if (live.length < 2) throw new Error("真实接口解析不足 2 点");
console.log("SMOKE OK");
```

Run: `node --experimental-strip-types <scratchpad>/smoke_index_quote.mts`
Expected: 打印 `live points: 30 last: { date: "2026-07-XX", close: 12XX.X }` 和 `SMOKE OK`，无 assert 报错。
注意：真实接口取到的 `lmt=60` 曾观察到在带 `beg=0` 时失效返回全量——本 URL 不带 `beg`，若 live points 仍远超 30 也没关系（`slice(-take)` 已截尾），只要 last 是最近交易日即可。

- [ ] **Step 1.3: Commit**

```bash
cd /Users/lyf/dev/coalassistant
git add doudou_blend/src/index_quote.ts
git commit -m "feat(today): 焦煤指数数据层 (东财日K + localStorage 缓存)"
```

---

### Task 2: UI 组件 `IndexTicker.tsx` + 挂载

**Files:**
- Create: `doudou_blend/src/IndexTicker.tsx`
- Modify: `doudou_blend/src/App.tsx`（import + today 分支）

- [ ] **Step 2.1: 写组件（完整内容如下）**

```tsx
/**
 * 焦煤指数窄条: 当前价 + 涨跌幅 + 近30日 sparkline.
 * 无数据(离线且无缓存)时整条不渲染. 只做展示, 不进任何求解逻辑.
 */
import { useEffect, useState } from "react";
import { fetchJmKline, type JmKline } from "./index_quote";

/** 内联 SVG 折线, 数据归一化到固定 viewBox, 不引图表库. */
function Sparkline({ closes, color }: { closes: number[]; color: string }) {
  const w = 72;
  const h = 20;
  const min = Math.min(...closes);
  const max = Math.max(...closes);
  const span = max - min || 1;
  const pts = closes
    .map((c, i) => {
      const x = (i / (closes.length - 1)) * w;
      const y = h - 2 - ((c - min) / span) * (h - 4);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden>
      <polyline points={pts} fill="none" stroke={color} strokeWidth={1.5} />
    </svg>
  );
}

export function IndexTicker() {
  const [kline, setKline] = useState<JmKline | null>(null);

  useEffect(() => {
    let alive = true;
    fetchJmKline().then((k) => {
      if (alive) setKline(k);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (!kline || kline.points.length < 2) return null;

  const closes = kline.points.map((p) => p.close);
  const last = closes[closes.length - 1];
  const prev = closes[closes.length - 2];
  const pct = ((last - prev) / prev) * 100;
  const up = pct >= 0;
  // 国内习惯: 红涨绿跌
  const color = up ? "var(--c-danger)" : "var(--c-success)";
  const lastDate = kline.points[kline.points.length - 1].date.slice(5); // MM-DD

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 2px",
        fontSize: 12,
        color: "var(--c-text-3)",
      }}
    >
      <span>焦煤JM主力</span>
      <span style={{ fontWeight: 700, fontSize: 14, color: "var(--c-text)" }}>
        {last.toFixed(1)}
      </span>
      <span style={{ color, fontWeight: 600 }}>
        {up ? "▲+" : "▼"}
        {pct.toFixed(2)}%
      </span>
      {kline.stale && <span style={{ fontSize: 10 }}>({lastDate})</span>}
      <span style={{ marginLeft: "auto", display: "flex" }}>
        <Sparkline closes={closes} color={color} />
      </span>
    </div>
  );
}
```

- [ ] **Step 2.2: App.tsx 挂载**

`doudou_blend/src/App.tsx`——加 import：

```tsx
import { IndexTicker } from "./IndexTicker";
```

today 分支（现第 30 行 `{tab === "today" && <TodayScreen onNavigate={setTab} />}`）改为：

```tsx
{tab === "today" && (
  <>
    <IndexTicker />
    <TodayScreen onNavigate={setTab} />
  </>
)}
```

（挂 App 层而非 TodayScreen 内部：TodayScreen 有 loading/error/不可行/成功 4 个 return 分支，挂这里一处改动全分支可见。）

- [ ] **Step 2.3: 类型检查**

Run: `cd /Users/lyf/dev/coalassistant/doudou_blend && npx tsc --noEmit`
Expected: 无输出（零错误）。

- [ ] **Step 2.4: Commit**

```bash
cd /Users/lyf/dev/coalassistant
git add doudou_blend/src/IndexTicker.tsx doudou_blend/src/App.tsx
git commit -m "feat(today): 今日屏顶部焦煤指数窄条 (当前价+涨跌+30日曲线)"
```

---

### Task 3: 双端验证（人工/浏览器）

**Files:** 无改动

- [ ] **Step 3.1: Web 端三态验证**

Run: `cd /Users/lyf/dev/coalassistant/doudou_blend && npm run dev`（Vite :1420）

浏览器打开 `http://localhost:1420`，登录（本地硬编码凭据见 `src/storage.ts` 的 `AUTH_USER`/`AUTH_PASS`），验证：
1. **正常态**：今日屏顶部出现「焦煤JM主力 12XX.X ▲/▼X.XX% + 曲线」，价格与东财 `114.jmm`（焦煤主连）当日一致
2. **stale 态**：DevTools → Network 切 Offline → 改 localStorage `doudou_blend.jm_kline.v1` 的 `fetchedAt` 为 1 小时前 → 刷新 → 窄条仍显示且带灰字 `(MM-DD)` 日期
3. **无数据态**：保持 Offline → 删掉 localStorage 该 key → 刷新 → 窄条消失，console 无未捕获报错

- [ ] **Step 3.2: 原生端验证**

Run: `cd /Users/lyf/dev/coalassistant/doudou_blend && npm run tauri dev`
Expected: 今日屏同样显示窄条（验证 Tauri webview 下 fetch 跨域可用——spec 已 curl 实测 CORS 头，这里确认 webview 行为一致）。

- [ ] **Step 3.3: 收尾**

若 3.1/3.2 有问题：修复 → 重跑对应验证 → `git commit -m "fix(today): <问题描述>"`。全部通过则任务结束（无需额外 commit）。
