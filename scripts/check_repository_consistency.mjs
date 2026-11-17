import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const failures = [];

function readRepositoryFile(relativePath) {
  return readFileSync(join(repositoryRoot, relativePath), "utf8");
}

function recordFailure(message) {
  failures.push(message);
}

function compareStringLists(label, rustValues, typescriptValues) {
  const rustSorted = [...rustValues].sort();
  const typescriptSorted = [...typescriptValues].sort();
  if (JSON.stringify(rustSorted) !== JSON.stringify(typescriptSorted)) {
    recordFailure(
      `${label} 不一致:\n  Rust: ${rustSorted.join(", ")}\n  TypeScript: ${typescriptSorted.join(", ")}`,
    );
  }
}

function extractDelimitedBlock(source, declarationPattern, label) {
  const declaration = declarationPattern.exec(source);
  if (!declaration) throw new Error(`找不到声明: ${label}`);
  const openingBrace = source.indexOf("{", declaration.index);
  let braceDepth = 0;
  for (let offset = openingBrace; offset < source.length; offset += 1) {
    if (source[offset] === "{") braceDepth += 1;
    if (source[offset] === "}") braceDepth -= 1;
    if (braceDepth === 0) return source.slice(openingBrace + 1, offset);
  }
  throw new Error(`声明未闭合: ${label}`);
}

function extractRustStructFields(source, structName) {
  const block = extractDelimitedBlock(
    source,
    new RegExp(`pub\\s+struct\\s+${structName}\\s*\\{`),
    `Rust struct ${structName}`,
  );
  // 带 #[serde(skip)] 的字段不上 JSON, 只在 Rust 内部传值, TypeScript 侧本就没有.
  return [
    ...block.matchAll(
      /^((?:[ \t]*#\[[^\n]*\][ \t]*\n)*)[ \t]*pub\s+([A-Za-z_][A-Za-z0-9_]*)\s*:/gm,
    ),
  ]
    .filter((fieldMatch) => !fieldMatch[1].includes("#[serde(skip)]"))
    .map((fieldMatch) => fieldMatch[2]);
}

function extractTypescriptInterfaceFields(source, interfaceName) {
  const block = extractDelimitedBlock(
    source,
    new RegExp(`export\\s+interface\\s+${interfaceName}\\s*\\{`),
    `TypeScript interface ${interfaceName}`,
  );
  return [...block.matchAll(/^\s*([A-Za-z_][A-Za-z0-9_]*)\??\s*:/gm)].map(
    (fieldMatch) => fieldMatch[1],
  );
}

function extractRustEnumVariants(source, enumName) {
  const block = extractDelimitedBlock(
    source,
    new RegExp(`pub\\s+enum\\s+${enumName}\\s*\\{`),
    `Rust enum ${enumName}`,
  );
  return [...block.matchAll(/^\s*([A-Z][A-Za-z0-9_]*)\s*(?:,|\(|\{)/gm)].map(
    (variantMatch) => variantMatch[1],
  );
}

function extractTypescriptUnionValues(source, typeName) {
  const unionMatch = new RegExp(`export\\s+type\\s+${typeName}\\s*=([\\s\\S]*?);`).exec(source);
  if (!unionMatch) throw new Error(`找不到 TypeScript union ${typeName}`);
  return [...unionMatch[1].matchAll(/["']([^"']+)["']/g)].map(
    (literalMatch) => literalMatch[1],
  );
}

function extractQuotedValues(sourceFragment) {
  return [...sourceFragment.matchAll(/["']([^"']+)["']/g)].map(
    (literalMatch) => literalMatch[1],
  );
}

/** 常量数组本体: `const NAME ... = [ ... ]` (Rust 的 `= &[ ... ]` 同形), 返回方括号内的文本. */
function extractConstArrayBody(source, constName, label) {
  const assignment = new RegExp(`\\b${constName}\\b[^=]*=\\s*&?\\s*\\[`).exec(source);
  if (!assignment) throw new Error(`找不到常量列表: ${label}`);
  const openingBracket = assignment.index + assignment[0].length - 1;
  let depth = 0;
  for (let offset = openingBracket; offset < source.length; offset += 1) {
    if (source[offset] === "[") depth += 1;
    if (source[offset] === "]") depth -= 1;
    if (depth === 0) return source.slice(openingBracket + 1, offset);
  }
  throw new Error(`常量列表未闭合: ${label}`);
}

/** 字面量列表比对: 两边都是 `const NAME = [ "a", "b" ]` 形的字符串列表, 抽全部引号字面量. */
function extractConstStringList(source, constName, label) {
  return extractQuotedValues(extractConstArrayBody(source, constName, label));
}

/** Rust 元组列表 `const NAME = &[ ("S", 0.0, 10.0), ... ]` 的每项首元素. */
function extractRustTupleFirstStrings(source, constName, label) {
  return [...extractConstArrayBody(source, constName, label).matchAll(/\(\s*"([^"]+)"/g)].map(
    (tupleMatch) => tupleMatch[1],
  );
}

/** 数值常量: `const NAME(: f64)? = 100_000.0;` (下划线分隔符去掉). */
function extractConstNumber(source, constName, label) {
  const declaration = new RegExp(`\\b${constName}\\b[^=]*=\\s*([0-9][0-9_.]*)`).exec(source);
  if (!declaration) throw new Error(`找不到数值常量: ${label}`);
  return Number(declaration[1].replaceAll("_", ""));
}

/** 两个数值常量必须相等. */
function compareConstNumbers(label, rustSource, typescriptSource, constName) {
  const rustValue = extractConstNumber(rustSource, constName, `Rust ${constName}`);
  const typescriptValue = extractConstNumber(typescriptSource, constName, `TypeScript ${constName}`);
  if (rustValue !== typescriptValue) {
    recordFailure(`${label} 不一致:\n  Rust: ${rustValue}\n  TypeScript: ${typescriptValue}`);
  }
}

function readCargoPackageVersion(relativePath) {
  const cargoSource = readRepositoryFile(relativePath);
  const packageBlock = /\[package\]([\s\S]*?)(?=\n\[|$)/.exec(cargoSource)?.[1];
  const version = packageBlock && /^version\s*=\s*"([^"]+)"/m.exec(packageBlock)?.[1];
  if (!version) throw new Error(`找不到 Cargo package version: ${relativePath}`);
  return version;
}

function readCargoPackageLicense(relativePath) {
  const cargoSource = readRepositoryFile(relativePath);
  const packageBlock = /\[package\]([\s\S]*?)(?=\n\[|$)/.exec(cargoSource)?.[1];
  const license = packageBlock && /^license\s*=\s*"([^"]+)"/m.exec(packageBlock)?.[1];
  return license ?? null;
}

function collectSourceFiles(directory) {
  const ignoredDirectories = new Set([".git", "dist", "node_modules", "pkg", "target"]);
  const allowedExtensions = new Set([".md", ".rs", ".ts", ".tsx"]);
  const collectedFiles = [];
  for (const entryName of readdirSync(directory)) {
    if (ignoredDirectories.has(entryName)) continue;
    const absolutePath = join(directory, entryName);
    if (statSync(absolutePath).isDirectory()) {
      collectedFiles.push(...collectSourceFiles(absolutePath));
    } else if (allowedExtensions.has(extname(entryName))) {
      collectedFiles.push(absolutePath);
    }
  }
  return collectedFiles;
}

const rustModelSource = readRepositoryFile("blend_kit_rs/src/model.rs");
const rustSeedSource = readRepositoryFile("blend_kit_rs/src/seed.rs");
const rustPetrographySource = readRepositoryFile("blend_kit_rs/src/petrography.rs");
const rustOptimizerSource = readRepositoryFile("blend_kit_rs/src/optimizer.rs");
const rustAdminSource = readRepositoryFile("blend_kit_server/src/admin.rs");
const rustBenchmarksSource = readRepositoryFile("blend_kit_server/src/benchmarks.rs");
const rustCoalPricesSource = readRepositoryFile("blend_kit_server/src/coal_prices.rs");
const rustDatabaseSource = readRepositoryFile("blend_kit_server/src/database.rs");
const rustMasterDataSource = readRepositoryFile("blend_kit_server/src/master_data.rs");
const rustQuotesSource = readRepositoryFile("blend_kit_server/src/quotes.rs");
const rustTodaySource = readRepositoryFile("blend_kit_server/src/today.rs");
const typescriptSource = readRepositoryFile("doudou_blend/src/types.ts");
const tsAdminSource = readRepositoryFile("doudou_blend/src/admin.ts");
const tsBenchmarksSource = readRepositoryFile("doudou_blend/src/benchmarks.ts");
const tsHistoryScreenSource = readRepositoryFile("doudou_blend/src/screens/HistoryScreen.tsx");
const tsQuoteScreenSource = readRepositoryFile("doudou_blend/src/screens/QuoteScreen.tsx");
const tsResultNotesSource = readRepositoryFile("doudou_blend/src/domain/resultNotes.ts");
const tsStorageSource = readRepositoryFile("doudou_blend/src/storage.ts");
const masterDataCheckSource = readRepositoryFile("scripts/check_master_data.mjs");

const sharedStructs = [
  [rustQuotesSource, "ArchivedQuote", "ArchivedQuote"],
  [rustQuotesSource, "LatestQuote", "LatestQuote"],
  [rustQuotesSource, "QuoteRecord", "QuoteRecord"],
  [rustQuotesSource, "QuoteTypeSummary", "QuoteTypeSummary"],

  [rustCoalPricesSource, "CoalPricePoint", "CoalPricePoint"],
  [rustCoalPricesSource, "CoalPriceHistory", "CoalPriceHistory"],
  [rustCoalPricesSource, "CoalPriceEntry", "CoalPriceEntry"],
  [rustCoalPricesSource, "CoalPriceSnapshot", "CoalPriceSnapshot"],
  [rustModelSource, "Coal", "Coal"],
  [rustModelSource, "AcceptanceRule", "AcceptanceRule"],
  [rustModelSource, "Spec", "Spec"],
  [rustModelSource, "BlendRequest", "BlendRequest"],
  [rustModelSource, "RankInteraction", "RankInteraction"],
  [rustModelSource, "CostBreakdown", "CostBreakdown"],
  [rustModelSource, "OrderItem", "OrderItem"],
  [rustModelSource, "ModelSummary", "ModelSummary"],
  [rustModelSource, "IndicatorCheck", "IndicatorCheck"],
  [rustModelSource, "PetrographyCheck", "PetrographyCheck"],
  [rustModelSource, "InfeasibleBound", "InfeasibleBound"],
  [rustModelSource, "BlendResult", "BlendResult"],
  [rustModelSource, "PenaltyTier", "PenaltyTier"],
  [rustModelSource, "Penalty", "Penalty"],
  [rustModelSource, "PurchaseClause", "PurchaseClause"],
  [rustModelSource, "PurchaseTerms", "PurchaseTerms"],
  [rustModelSource, "PriceAdjust", "PriceAdjust"],
  [rustModelSource, "PriceFactor", "PriceFactor"],
  [rustModelSource, "PricePart", "PricePart"],
  [rustModelSource, "CoalValue", "CoalValue"],
  [rustPetrographySource, "Petrography", "Petrography"],
  [rustPetrographySource, "Notch", "Notch"],
  [rustModelSource, "PriceRobustness", "PriceRobustness"],
  [rustModelSource, "MatchConfig", "MatchConfig"],
  [rustModelSource, "Benchmark", "Benchmark"],
  [rustModelSource, "PriceResult", "PriceResult"],
  [rustModelSource, "PriceUncertainty", "PriceUncertainty"],
  [rustModelSource, "BenchmarkShare", "BenchmarkShare"],
  [rustModelSource, "RobustnessReport", "RobustnessReport"],
  [rustModelSource, "PriceSwing", "PriceSwing"],
  [rustModelSource, "RobustRecipe", "RobustRecipe"],
  [rustDatabaseSource, "MeasuredQuality", "MeasuredQuality"],
  [rustSeedSource, "CoalMasterEntry", "MasterCoalEntry"],
  [rustSeedSource, "DefaultContract", "DefaultContract"],
  [rustSeedSource, "CoalMaster", "CoalMaster"],
];

for (const [rustSource, rustName, typescriptName] of sharedStructs) {
  compareStringLists(
    `${rustName}/${typescriptName} 字段`,
    extractRustStructFields(rustSource, rustName),
    extractTypescriptInterfaceFields(typescriptSource, typescriptName),
  );
}

compareStringLists(
  "Direction 枚举",
  extractRustEnumVariants(rustModelSource, "Direction"),
  extractTypescriptUnionValues(typescriptSource, "Direction"),
);
for (const enumName of [
  "AcceptanceMode",
  "Enforcement",
  "EvaluationMethod",
  "EvaluationStatus",
  "QualityStatus",
  "ModelKind",
]) {
  compareStringLists(
    `${enumName} 枚举`,
    extractRustEnumVariants(rustModelSource, enumName),
    extractTypescriptUnionValues(typescriptSource, enumName),
  );
}
compareStringLists(
  "MasterStatus/CoalStatus 枚举",
  extractRustEnumVariants(rustSeedSource, "MasterStatus").map((variant) => variant.toLowerCase()),
  // CoalStatus 由 COAL_STATUSES 常量数组派生 ((typeof ...)[number]), 词表读常量.
  extractConstStringList(typescriptSource, "COAL_STATUSES", "types.ts COAL_STATUSES"),
);
compareStringLists(
  "Confidence 枚举",
  extractRustEnumVariants(rustSeedSource, "Confidence").map((variant) => variant.toLowerCase()),
  extractTypescriptUnionValues(typescriptSource, "Confidence"),
);

const rustIndicatorsDeclaration = /pub const INDICATORS[^=]*=\s*\[([^\]]+)\]/.exec(
  rustModelSource,
);
const typescriptIndicatorOrder = /export const INDICATOR_ORDER[^=]*=\s*\[([^\]]+)\]/.exec(
  typescriptSource,
);
if (!rustIndicatorsDeclaration || !typescriptIndicatorOrder) {
  recordFailure("无法读取 Rust/TypeScript 指标顺序");
} else {
  const rustIndicators = extractQuotedValues(rustIndicatorsDeclaration[1]);
  const typescriptIndicators = extractQuotedValues(typescriptIndicatorOrder[1]);
  if (JSON.stringify(rustIndicators) !== JSON.stringify(typescriptIndicators)) {
    recordFailure(
      `八项指标顺序不一致:\n  Rust: ${rustIndicators.join(", ")}\n  TypeScript: ${typescriptIndicators.join(", ")}`,
    );
  }
  compareStringLists(
    "IndicatorKey 联合类型",
    rustIndicators,
    extractTypescriptUnionValues(typescriptSource, "IndicatorKey"),
  );
}

// ---------------------------------------------------------------------------
// 字面量列表比对: 前端 / 服务端 / 脚本各抄一份的常量, 必须逐份对齐.
// ---------------------------------------------------------------------------

compareStringLists(
  "MASTER_ONLY_KEYS (benchmarks.rs ↔ benchmarks.ts)",
  extractConstStringList(rustBenchmarksSource, "MASTER_ONLY_KEYS", "benchmarks.rs MASTER_ONLY_KEYS"),
  extractConstStringList(tsBenchmarksSource, "MASTER_ONLY_KEYS", "benchmarks.ts MASTER_ONLY_KEYS"),
);
compareStringLists(
  "OVERRIDE_FIELDS (admin.ts ↔ master_data.rs INDICATOR_RANGE)",
  extractRustTupleFirstStrings(rustMasterDataSource, "INDICATOR_RANGE", "master_data.rs INDICATOR_RANGE"),
  extractConstStringList(tsAdminSource, "OVERRIDE_FIELDS", "admin.ts OVERRIDE_FIELDS"),
);
compareStringLists(
  "CONFIDENCE_LEVELS (admin.ts ↔ master_data.rs CONFIDENCES)",
  extractConstStringList(rustMasterDataSource, "CONFIDENCES", "master_data.rs CONFIDENCES"),
  extractConstStringList(tsAdminSource, "CONFIDENCE_LEVELS", "admin.ts CONFIDENCE_LEVELS"),
);

// INDICATOR_LABEL (types.ts) ↔ model.rs label_zh 的 match 臂 ("S" => "硫" …)
const rustLabels = [
  ...extractDelimitedBlock(rustModelSource, /pub\s+fn\s+label_zh\s*\(/, "model.rs label_zh").matchAll(
    /"([^"]+)"\s*=>\s*"([^"]+)"/g,
  ),
].map((arm) => `${arm[1]}=${arm[2]}`);
const typescriptLabels = [
  ...extractDelimitedBlock(
    typescriptSource,
    /export\s+const\s+INDICATOR_LABEL\b[^=]*=/,
    "types.ts INDICATOR_LABEL",
  ).matchAll(/([A-Za-z]+)\s*:\s*"([^"]+)"/g),
].map((entry) => `${entry[1]}=${entry[2]}`);
compareStringLists("INDICATOR_LABEL ↔ label_zh", rustLabels, typescriptLabels);

// 煤的状态词表三处一致: 前端 types.ts、服务端 today.rs、数据自检脚本.
const typescriptStatuses = extractConstStringList(
  typescriptSource,
  "COAL_STATUSES",
  "types.ts COAL_STATUSES",
);
compareStringLists(
  "COAL_STATUSES (types.ts ↔ today.rs)",
  extractConstStringList(rustTodaySource, "COAL_STATUSES", "today.rs COAL_STATUSES"),
  typescriptStatuses,
);
compareStringLists(
  "COAL_STATUSES (types.ts ↔ check_master_data.mjs STATUSES)",
  extractConstStringList(masterDataCheckSource, "STATUSES", "check_master_data.mjs STATUSES"),
  typescriptStatuses,
);

// 询价输入上限: 前端先拦, 服务端兜底, 数字必须一样.
compareConstNumbers("MAX_FOB (QuoteScreen ↔ quotes.rs)", rustQuotesSource, tsQuoteScreenSource, "MAX_FOB");
compareConstNumbers("MAX_FRT (QuoteScreen ↔ quotes.rs)", rustQuotesSource, tsQuoteScreenSource, "MAX_FRT");

// 历史回填量程: 界面输入框的量程与服务端 MeasuredQuality::validate 一致.
const typescriptRanges = [
  ...tsHistoryScreenSource.matchAll(
    /\[\s*"([a-z0-9_]+)"\s*,\s*"[^"]*"\s*,\s*"[^"]*"\s*,\s*([0-9.]+)\s*,\s*([0-9.]+)\s*,/g,
  ),
].map((field) => `${field[1]}=${Number(field[2])}~${Number(field[3])}`);
const percentDeclaration = /const\s+PERCENT[^=]*=\s*\(\s*([0-9.]+)\s*,\s*([0-9.]+)\s*,/.exec(
  rustDatabaseSource,
);
if (!percentDeclaration) throw new Error("找不到 database.rs 的 PERCENT 量程");
const measuredQualityImpl = extractDelimitedBlock(
  rustDatabaseSource,
  /impl\s+MeasuredQuality\s*\{/,
  "impl MeasuredQuality",
);
const rustRanges = [
  ...measuredQualityImpl.matchAll(
    /\(\s*self\.([a-z0-9_]+)\s*,\s*(?:PERCENT|\(\s*([0-9.]+)\s*,\s*([0-9.]+)\s*,)/g,
  ),
].map(
  (check) =>
    `${check[1]}=${check[2] === undefined ? Number(percentDeclaration[1]) : Number(check[2])}~${
      check[3] === undefined ? Number(percentDeclaration[2]) : Number(check[3])
    }`,
);
compareStringLists("历史回填量程 (HistoryScreen ↔ MeasuredQuality::validate)", rustRanges, typescriptRanges);

compareStringLists(
  "MIXED_KEYS (types.ts ↔ admin.rs)",
  extractConstStringList(rustAdminSource, "MIXED_KEYS", "admin.rs MIXED_KEYS"),
  extractConstStringList(typescriptSource, "MIXED_KEYS", "types.ts MIXED_KEYS"),
);
// today.rs 曾经复制过一份指标顺序, 现在直接用核心的 blend_kit::INDICATORS; 副本不许再长回来.
if (/\bconst\s+INDICATORS\b/.test(rustTodaySource)) {
  recordFailure(
    "blend_kit_server/src/today.rs 又定义了自己的 INDICATORS: 指标顺序只有核心 model.rs 一份, 用 blend_kit::INDICATORS",
  );
}
compareConstNumbers(
  "DEFAULT_SHOVEL_TONS (database.rs ↔ storage.ts)",
  rustDatabaseSource,
  tsStorageSource,
  "DEFAULT_SHOVEL_TONS",
);

// 约束冲突的 reason 原文: 前端按原文识别与展示, 服务端改字这里就会拦.
const conflictReason = /CONFLICT_REASON\s*:\s*&str\s*=\s*"([^"]+)"/.exec(rustOptimizerSource)?.[1];
if (!conflictReason) throw new Error("找不到 optimizer.rs 的 CONFLICT_REASON");
if (!tsResultNotesSource.includes(conflictReason)) {
  recordFailure(`resultNotes.ts 没有与 CONFLICT_REASON 一致的原文 "${conflictReason}"`);
}

const versionEntries = [
  ["blend_kit_rs/Cargo.toml", readCargoPackageVersion("blend_kit_rs/Cargo.toml")],
  ["blend_kit_wasm/Cargo.toml", readCargoPackageVersion("blend_kit_wasm/Cargo.toml")],
  ["blend_kit_server/Cargo.toml", readCargoPackageVersion("blend_kit_server/Cargo.toml")],
  ["doudou_blend/package.json", JSON.parse(readRepositoryFile("doudou_blend/package.json")).version],
];
const canonicalVersion = versionEntries[0][1];
for (const [relativePath, version] of versionEntries) {
  if (version !== canonicalVersion) {
    recordFailure(`${relativePath} 版本为 ${version}，应为 ${canonicalVersion}`);
  }
}

const master = JSON.parse(readRepositoryFile("blend_kit_rs/data/coal_master.json"));
const masterEntryCount = master.coals.length;

const staticMasterPath = join(repositoryRoot, "doudou_blend/public/coal_master.json");
if (existsSync(staticMasterPath)) {
  recordFailure("存在重复的 doudou_blend/public/coal_master.json，应只保留 Rust Master 源");
}

const coreLibrarySource = readRepositoryFile("blend_kit_rs/src/lib.rs");
const wasmLibrarySource = readRepositoryFile("blend_kit_wasm/src/lib.rs");
const serverLibrarySource = readRepositoryFile("blend_kit_server/src/lib.rs");
const backendSource = readRepositoryFile("doudou_blend/src/backend.ts");
if (!/pub\s+fn\s+master_json\s*\(/.test(coreLibrarySource)) {
  recordFailure("blend_kit 未提供统一 master_json() 入口");
}
if (!/blend_kit::master_json\s*\(/.test(wasmLibrarySource)) {
  recordFailure("WASM 未复用 blend_kit::master_json()");
}
if (!/blend_kit::master_json\s*\(/.test(serverLibrarySource)) {
  recordFailure("Web 服务端未复用 blend_kit::master_json()");
}
if (!/["']\/api\/master["']/.test(serverLibrarySource)) {
  recordFailure("Web 服务端未暴露 /api/master 路由");
}
if (!/requestApi\(\s*["']master["']\s*\)/.test(backendSource)) {
  recordFailure("前端未通过 /api/master 读取统一 Master");
}

const licensePath = join(repositoryRoot, "LICENSE");
if (!existsSync(licensePath)) {
  recordFailure("缺少 LICENSE 文件");
} else {
  const licenseSource = readFileSync(licensePath, "utf8");
  if (!licenseSource.includes("MIT License") || !licenseSource.includes("Permission is hereby granted")) {
    recordFailure("LICENSE 不是完整的 MIT License 文本");
  }
  const wasmLicensePath = join(repositoryRoot, "blend_kit_wasm/LICENSE");
  if (!existsSync(wasmLicensePath)) {
    recordFailure("blend_kit_wasm 缺少随发布包分发的 LICENSE");
  } else if (readFileSync(wasmLicensePath, "utf8") !== licenseSource) {
    recordFailure("blend_kit_wasm/LICENSE 与仓库根 LICENSE 不一致");
  }
}
const packageManifest = JSON.parse(readRepositoryFile("doudou_blend/package.json"));
const packageLock = JSON.parse(readRepositoryFile("doudou_blend/package-lock.json"));
const licenseEntries = [
  ["blend_kit_rs/Cargo.toml", readCargoPackageLicense("blend_kit_rs/Cargo.toml")],
  ["blend_kit_wasm/Cargo.toml", readCargoPackageLicense("blend_kit_wasm/Cargo.toml")],
  [
    "blend_kit_server/Cargo.toml",
    readCargoPackageLicense("blend_kit_server/Cargo.toml"),
  ],
  ["doudou_blend/package.json", packageManifest.license ?? null],
  ["doudou_blend/package-lock.json", packageLock.packages?.[""]?.license ?? null],
];
for (const [relativePath, license] of licenseEntries) {
  if (license !== "MIT") recordFailure(`${relativePath} 未声明 MIT license`);
}

for (const sourcePath of collectSourceFiles(repositoryRoot)) {
  const source = readFileSync(sourcePath, "utf8");
  if (/\b73(?:\+)?\s*(?:种煤|煤种|种)/.test(source)) {
    recordFailure(`${sourcePath.slice(repositoryRoot.length + 1)} 仍包含过期的 73 种煤描述`);
  }
}

if (failures.length > 0) {
  console.error(`仓库一致性检查失败 (${failures.length}):`);
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log(`仓库一致性检查通过: ${masterEntryCount} 条 Master 目录记录，版本 ${canonicalVersion}`);
