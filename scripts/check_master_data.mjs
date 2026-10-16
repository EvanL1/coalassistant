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
// 期望的 status 取值. 真源是 master.schema.status, 这里只作交叉校验:
// 两边不一致说明有人只改了一边.
const STATUSES = ["verified", "active", "draft", "incomplete", "archived"];
const CONFIDENCES = ["high", "medium", "low", "spec"];

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

// ---------- 受控词表 (真源在 schema, 加一个煤种只改数据文件) ----------
const schema = master.schema ?? {};
const schemaStatuses = Object.keys(schema.status ?? {});
if (schemaStatuses.join("|") !== STATUSES.join("|")) {
  fail(`schema.status 与校验脚本不一致: schema=${schemaStatuses.join("/")} 脚本=${STATUSES.join("/")}`);
}
const COAL_TYPES = Object.keys(schema.coal_type ?? {});
const FORMS = Object.keys(schema.form ?? {});
if (COAL_TYPES.length === 0) fail("schema.coal_type 未声明煤种词表");
if (FORMS.length === 0) fail("schema.form 未声明产品形态词表");

/**
 * GB/T 5751-2009 烟煤分类, 按 schema.coal_type_basis: V 视为 Vdaf, 判据 V/G/Y.
 * G > 85 且 Y > 25 才归肥煤 / 气肥煤; 缺 Y 时按焦煤或 1/3焦煤 记 (同 basis 的约定).
 * 返回 null = 指标不够, 判不了.
 */
function gbCoalType(props) {
  const { V, G, Y } = props;
  if (typeof V !== "number" || typeof G !== "number") return null;
  if (V <= 10) return "无烟煤";
  const fat = G > 85 && typeof Y === "number" && Y > 25;
  if (V <= 20) {
    if (fat) return "肥煤";
    if (G <= 5) return "贫煤";
    if (G <= 20) return "贫瘦煤";
    return G <= 65 ? "瘦煤" : "焦煤";
  }
  if (V <= 37 && !fat) {
    if (G <= 5) return "不粘煤";
    if (G <= 30) return "弱粘煤";
    if (G <= 50) return "1/2中粘煤";
    if (V <= 28) return "焦煤";
    return G <= 65 ? "气煤" : "1/3焦煤";
  }
  if (V <= 37) return "肥煤";
  if (fat) return "气肥煤";
  return G > 35 ? "气煤" : "长焰煤";
}

// 煤种与国标判据不一致只提示, 不判失败: 有的是用户按实情改标 (如庞庞塔), 有的是
// Y 只有规格值、真实值可能不过 25, 要人看.
const notices = [];

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

  // coal_type / form 必须落在 schema 声明的词表里 —— 这两列曾混入
  // 「主焦煤」(贸易口语) 与「煤泥」(产品形态), 没有校验就会再漂回去.
  if (coal.coal_type != null && !COAL_TYPES.includes(coal.coal_type)) {
    fail(`${label}: coal_type ${JSON.stringify(coal.coal_type)} 不在 schema.coal_type 词表内`);
  }
  if (coal.form != null && !FORMS.includes(coal.form)) {
    fail(`${label}: form ${JSON.stringify(coal.form)} 不在 schema.form 词表内`);
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
  // 能进煤池的煤必须标煤种: 价格稳健性按煤种取估价误差, 今日屏和手机 Agent 按煤种展示.
  if (["verified", "active"].includes(coal.status)) {
    if (coal.coal_type == null) fail(`${label}: status=${coal.status} 却没有 coal_type`);
    const gb = gbCoalType(props);
    if (gb && coal.coal_type != null && gb !== coal.coal_type) {
      const spec = coal.confidence?.Y === "spec" ? " (Y 是规格值)" : "";
      notices.push(
        `${label}: 标 ${coal.coal_type}, 国标判据为 ${gb} (V${props.V} G${props.G} Y${props.Y ?? "缺"}${spec})`,
      );
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

// ---------- 价格因子系数表 (可选) ----------
const priceFactor = master.price_factor;
if (priceFactor != null) {
  const adjusts = priceFactor.adjusts;
  if (!Array.isArray(adjusts)) {
    fail("price_factor.adjusts 必须是数组");
  } else {
    const isRate = (v) => typeof v === "number" && Number.isFinite(v) && v >= 0;
    const isInterval = (v) =>
      v == null ||
      (Array.isArray(v) && v.length === 2 && v.every(Number.isFinite) && v[0] < v[1]);
    for (const rule of adjusts) {
      const label = `price_factor ${rule.indicator}`;
      if (!INDICATORS.includes(rule.indicator)) fail(`${label}: 未知指标`);
      if (rule.direction !== "Upper" && rule.direction !== "Lower") {
        fail(`${label}: direction 只能是 Upper/Lower, 实际 ${JSON.stringify(rule.direction)}`);
      }
      if (typeof rule.base !== "number" || !Number.isFinite(rule.base)) fail(`${label}: base 不是有限数值`);
      if (!isRate(rule.premium_rate)) fail(`${label}: premium_rate 须为 ≥0 的有限数`);
      if (!isRate(rule.discount_rate)) fail(`${label}: discount_rate 须为 ≥0 的有限数`);
      if (!isInterval(rule.clamp)) fail(`${label}: clamp 须为 [lo, hi] 且 lo < hi`);
      if (!isInterval(rule.linear_range)) fail(`${label}: linear_range 须为 [lo, hi] 且 lo < hi`);
    }
    const duplicated = adjusts
      .map((r) => r.indicator)
      .filter((ind, i, all) => all.indexOf(ind) !== i);
    if (duplicated.length) fail(`price_factor: 指标重复 ${[...new Set(duplicated)].join("/")}`);
  }
  const uncertainty = priceFactor.uncertainty;
  if (uncertainty != null) {
    const isError = (v) => typeof v === "number" && Number.isFinite(v) && v >= 0;
    if (!isError(uncertainty.default)) fail("price_factor.uncertainty.default 须为 ≥0 的有限数");
    if (!isError(uncertainty.quoted)) fail("price_factor.uncertainty.quoted 须为 ≥0 的有限数");
    for (const [type, value] of Object.entries(uncertainty.by_coal_type ?? {})) {
      if (!COAL_TYPES.includes(type)) fail(`price_factor.uncertainty: 煤种 ${type} 不在 schema.coal_type 词表内`);
      if (!isError(value)) fail(`price_factor.uncertainty.${type} 须为 ≥0 的有限数`);
    }
  }
}

// ---------- 结果 ----------
if (failures.length) {
  console.error(`Master 数据自检失败 (${failures.length} 项):`);
  for (const message of failures) console.error(`  - ${message}`);
  process.exit(1);
}

if (notices.length) {
  console.log(`煤种与国标判据不一致 (${notices.length} 条, 仅提示, 请人工确认):`);
  for (const message of notices) console.log(`  - ${message}`);
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
