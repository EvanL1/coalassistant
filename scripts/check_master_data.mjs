#!/usr/bin/env node
/**
 * coal_master.json 数据自检.
 *
 * 职责边界: 这里只校验**数据本身**是否自洽, 不碰任何业务代码.
 * 与之对应, Rust 侧的单元测试只测代码行为 (用 fixture), 不再断言数据内容 ——
 * 所以更新煤库数据永远不需要改 Rust 代码, 只需要这个脚本仍然通过.
 *
 * 用法: node scripts/check_master_data.mjs
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const MASTER_PATH = "blend_kit_rs/data/coal_master.json";

const failures = [];
const fail = (message) => failures.push(message);

const master = JSON.parse(readFileSync(join(repositoryRoot, MASTER_PATH), "utf8"));

/** 8 项化验指标及其物理量程 (超出即录入错误, 与具体煤源无关). */
const INDICATOR_RANGE = {
  S: [0, 10],
  A: [0, 50],
  V: [0, 50],
  G: [0, 100],
  Y: [0, 50],
  petro: [0, 1],
  CSR: [0, 100],
  M: [0, 30],
};
const INDICATORS = Object.keys(INDICATOR_RANGE);
const STATUSES = ["verified", "active", "draft", "incomplete", "archived"];
const CONFIDENCES = ["high", "medium", "low"];

// ---------- 顶层元数据 ----------
if (!/^\d+\.\d+$/.test(master.version ?? "")) {
  fail(`version 应形如 "2.2", 实际 ${JSON.stringify(master.version)}`);
}
if (!/^\d{4}-\d{2}-\d{2}$/.test(master.updated_at ?? "")) {
  fail(`updated_at 应形如 "2026-09-15", 实际 ${JSON.stringify(master.updated_at)}`);
}
if (!Array.isArray(master.coals) || master.coals.length === 0) {
  fail("coals 必须是非空数组");
}

// 防误删兜底: 煤库规模不该一夜腰斩 (数据层的护栏, 不是业务断言).
const MIN_COALS = 60;
if (master.coals.length < MIN_COALS) {
  fail(`煤源仅 ${master.coals.length} 条, 低于下限 ${MIN_COALS} —— 疑似误删`);
}

// ---------- 逐条记录 ----------
const seen = new Set();
for (const coal of master.coals) {
  const label = coal.name ?? "(无名)";

  if (!coal.name) fail("存在无 name 的记录");
  if (seen.has(coal.name)) fail(`煤名重复: ${coal.name}`);
  seen.add(coal.name);

  if (!STATUSES.includes(coal.status)) {
    fail(`${label}: status ${JSON.stringify(coal.status)} 不在 ${STATUSES.join("/")} 内`);
  }

  const props = coal.props ?? {};
  for (const [key, value] of Object.entries(props)) {
    if (!INDICATORS.includes(key)) {
      fail(`${label}: props 含未知指标 ${key}`);
      continue;
    }
    if (typeof value !== "number" || !Number.isFinite(value)) {
      fail(`${label}: ${key} 不是有限数值 (${JSON.stringify(value)})`);
      continue;
    }
    const [min, max] = INDICATOR_RANGE[key];
    if (value < min || value > max) {
      fail(`${label}: ${key}=${value} 超出物理量程 [${min}, ${max}]`);
    }
  }

  for (const [key, level] of Object.entries(coal.confidence ?? {})) {
    if (!INDICATORS.includes(key) && key !== "fob" && key !== "frt") {
      fail(`${label}: confidence 含未知字段 ${key}`);
    }
    if (!CONFIDENCES.includes(level)) {
      fail(`${label}: confidence.${key}=${level} 不在 ${CONFIDENCES.join("/")} 内`);
    }
  }

  // status 语义约束 —— 这些对任何一份合法数据都必须成立.
  if (coal.status === "verified") {
    const missing = INDICATORS.filter((k) => !(k in props));
    if (missing.length) fail(`${label}: verified 却缺指标 ${missing.join("/")}`);
    if (typeof coal.fob !== "number") fail(`${label}: verified 却没有 fob`);
    if (typeof coal.frt !== "number") fail(`${label}: verified 却没有 frt`);
  }
  if (coal.status === "incomplete" && Object.keys(props).length > 0) {
    fail(`${label}: incomplete 不应带 props (实际 ${Object.keys(props).join("/")})`);
  }
  // 有 S/A/V/G 四项基础指标才可能进煤池; incomplete 之外的煤若缺基础项, 标出来.
  if (["verified", "active", "draft"].includes(coal.status)) {
    const base = ["S", "A", "V", "G"].filter((k) => !(k in props));
    if (base.length === 4) {
      fail(`${label}: status=${coal.status} 却一项基础指标都没有`);
    }
  }
}

// ---------- 默认合同 ----------
const contract = master.default_contract;
if (!contract || !Array.isArray(contract.specs) || contract.specs.length === 0) {
  fail("default_contract.specs 必须是非空数组");
} else {
  for (const spec of contract.specs) {
    if (!INDICATORS.includes(spec.indicator)) {
      fail(`默认合同: 未知指标 ${spec.indicator}`);
    }
    const hasMin = typeof spec.min === "number";
    const hasMax = typeof spec.max === "number";
    if (spec.direction === "Upper" && !hasMax) fail(`默认合同 ${spec.indicator}: Upper 必须有 max`);
    if (spec.direction === "Lower" && !hasMin) fail(`默认合同 ${spec.indicator}: Lower 必须有 min`);
    if (spec.direction === "Range" && !(hasMin && hasMax)) {
      fail(`默认合同 ${spec.indicator}: Range 必须同时有 min/max`);
    }
    if (hasMin && hasMax && spec.min > spec.max) {
      fail(`默认合同 ${spec.indicator}: min ${spec.min} > max ${spec.max}`);
    }
  }
  const duplicated = contract.specs
    .map((s) => s.indicator)
    .filter((ind, i, all) => all.indexOf(ind) !== i);
  if (duplicated.length) fail(`默认合同: 指标重复 ${[...new Set(duplicated)].join("/")}`);
}

// ---------- 结果 ----------
if (failures.length) {
  console.error(`Master 数据自检失败 (${failures.length} 项):`);
  for (const message of failures) console.error(`  - ${message}`);
  process.exit(1);
}

const byStatus = {};
for (const coal of master.coals) byStatus[coal.status] = (byStatus[coal.status] ?? 0) + 1;
const full = master.coals.filter((c) => INDICATORS.every((k) => k in (c.props ?? {}))).length;
console.log(
  `Master 数据自检通过: ${master.coals.length} 条 (v${master.version}, ${master.updated_at}), ` +
    `8 项齐全 ${full} 条, ` +
    Object.entries(byStatus)
      .map(([k, v]) => `${k} ${v}`)
      .join(" / "),
);
