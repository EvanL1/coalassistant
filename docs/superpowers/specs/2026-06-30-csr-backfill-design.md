# 数据闭环 ① 回填实测焦质（只采集 + 回填）

- 日期：2026-06-30
- 分支：`feat/csr-backfill`
- 状态：设计已批准，待实现

## 背景

`blend_kit_rs/src/predict.rs` 的 CSR 线性回归引擎已就绪并接入 `solve()`，但缺数据可吃：
应用里没有任何入口让用户记录「一次真实配煤 → 实测焦炭 CSR」的观测对。本期补上这一环——
即数据闭环的第一步，把护城河数据采集起来。

## 目标与范围

- **采集**：保存配煤方案时，把完整结果落库（native → SQLite，web → localStorage）。
  回归所需的自变量 X（混合后 6 项指标 S/A/V/G/Y/M）随 `result_json` 一起留存。
- **回填**：在「历史方案」页，每条记录可录入实测 CSR（因变量 y）。
- **非目标（本期不做）**：
  - 不把观测喂进 `solveJson`（不改当前配煤推荐）。
  - 不做 G/Y 非加性校正。
  - 不动预测引擎本身。
  - 不引入前端测试框架（保持项目现状）。

## 架构：历史持久化搬到 `Backend` 接口后面

改用 SQLite 后，历史不能再在界面里同步读 localStorage，必须搬到异步 `Backend`
接口（`doudou_blend/src/backend.ts`）后面，和 `solveJson` 一致。屏幕代码对运行时无感。
按 CLAUDE.md 规矩，每个能力在两端各自实现。

| 方法 | `makeTauriBackend`（native） | `makeWasmBackend`（web） |
|------|------------------------------|--------------------------|
| `saveHistory(resultJson, contractName, costCif, quantity)` → `id` | `invoke('save_history', …)` → SQLite INSERT | `storage.ts` 写 localStorage |
| `listHistory()` → `HistoryRecord[]` | `invoke('list_history')` | 读 localStorage |
| `setMeasuredCsr(id, value)` | `invoke('set_measured_csr', …)` | 改 localStorage |

**边界 DTO 统一**：两端 `listHistory()` 返回同一形状
`HistoryRecord = { id, occurred_at, contract_name, cost_cif, result_json, csr_measured: number | null }`。
Rust 侧只存/取不透明 blob + csr 列，不依赖 `BlendResult` 内部结构；解析 `result_json`
（取 recipe、从 `indicator_check` 抽 6 项混合指标）由 TS 适配层统一做（`types.ts` 本就有镜像）。

## 数据模型 / schema 改动

- `db_schema.rs`：`blend_history` 的 `CREATE TABLE` 里加两列
  `csr_measured REAL`、`contract_name TEXT`（覆盖全新安装）。`contract_id` 外键留空。
- `db.rs` `init_schema`：`execute_batch(SCHEMA_V1)` 之后加幂等迁移 `add_column_if_missing`
  （查 `PRAGMA table_info(blend_history)`，缺列才 `ALTER TABLE ADD COLUMN`）——覆盖已有老库升级。
- web `storage.ts`：`HistoryEntry` 增加 `result?: BlendResult`、`csr_measured?: number`。

## 数据流

- **采集**：`TodayScreen.saveToHistory()` 改为
  `await backend.saveHistory(JSON.stringify(state.result), contractName, costCif, quantity)`。
  X 随 `result_json` 落库。
- **回填**：历史页录入框 → `await backend.setMeasuredCsr(id, value)` → 刷新列表。
  观测 = `result_json` 里的 6 项混合指标 + `csr_measured`，闭环存好（差以后喂）。

## UI（历史页内联，不用弹窗）

每条卡片底部加一行：
- `csr_measured == null` → 「录入实测CSR」按钮 → 内联数字输入 + 保存。
- 已录入 → 显示「实测 CSR 65.3 ✎」，可点改。
- 老记录（无 `result`/混合指标）→ 不显示录入入口（存了 y 也没 X，无意义）。

`HistoryScreen` 改异步：`useEffect` 里 `backend.listHistory()` 加载。

## 错误处理 & 测试

- 错误：输入校验（CSR 为正数、合理量程提示）；`DbError` 沿用现有 `serde` 序列化路径回传前端。
- 测试（Rust 侧 TDD，照 `db.rs` 现有 `TempDir` 套路）：
  - `save_history` / `list_history` / `set_measured_csr` 往返。
  - 迁移幂等：老库 ALTER 不报错、新库不重复加列。
- web localStorage 路径 + UI：在真实 app 里手验（与项目现状一致）。

## 已知后续项 (本期不做)

- `list_history` 每次返回完整 `result_json` (每条数 KB), 历史多时 IPC 负载偏大。
  当前历史规模小 (web 上限 100 条), 暂可接受。后续可拆 `recipe` 独立列, 或加
  `get_history_detail(id)` 按需取详情。

## 涉及文件

- `doudou_blend/src-tauri/src/db_schema.rs` — 加列
- `doudou_blend/src-tauri/src/db.rs` — 幂等迁移
- `doudou_blend/src-tauri/src/db_queries.rs` — 3 个查询函数 + 测试
- `doudou_blend/src-tauri/src/lib.rs` — 3 个 `#[tauri::command]`
- `doudou_blend/src/backend.ts` — 接口 + 两端实现
- `doudou_blend/src/storage.ts` — web 持久化扩展
- `doudou_blend/src/types.ts` — `HistoryRecord` DTO
- `doudou_blend/src/screens/TodayScreen.tsx` — 采集接线
- `doudou_blend/src/screens/HistoryScreen.tsx` — 异步加载 + 回填 UI
