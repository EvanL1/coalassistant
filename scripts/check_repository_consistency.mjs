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
  return [...block.matchAll(/^\s*pub\s+([A-Za-z_][A-Za-z0-9_]*)\s*:/gm)].map(
    (fieldMatch) => fieldMatch[1],
  );
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
const rustPredictSource = readRepositoryFile("blend_kit_rs/src/predict.rs");
const rustPetrographySource = readRepositoryFile("blend_kit_rs/src/petrography.rs");
const typescriptSource = readRepositoryFile("doudou_blend/src/types.ts");

const sharedStructs = [
  [rustModelSource, "Coal", "Coal"],
  [rustModelSource, "Spec", "Spec"],
  [rustModelSource, "BlendRequest", "BlendRequest"],
  [rustModelSource, "CostBreakdown", "CostBreakdown"],
  [rustModelSource, "OrderItem", "OrderItem"],
  [rustModelSource, "IndicatorCheck", "IndicatorCheck"],
  [rustModelSource, "PetrographyCheck", "PetrographyCheck"],
  [rustModelSource, "BlendResult", "BlendResult"],
  [rustPredictSource, "CsrObservation", "CsrObservation"],
  [rustPetrographySource, "Petrography", "Petrography"],
  [rustPetrographySource, "Notch", "Notch"],
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
compareStringLists(
  "MasterStatus/CoalStatus 枚举",
  extractRustEnumVariants(rustSeedSource, "MasterStatus").map((variant) => variant.toLowerCase()),
  extractTypescriptUnionValues(typescriptSource, "CoalStatus"),
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

const versionEntries = [
  ["blend_kit_rs/Cargo.toml", readCargoPackageVersion("blend_kit_rs/Cargo.toml")],
  ["blend_kit_wasm/Cargo.toml", readCargoPackageVersion("blend_kit_wasm/Cargo.toml")],
  ["doudou_blend/src-tauri/Cargo.toml", readCargoPackageVersion("doudou_blend/src-tauri/Cargo.toml")],
  ["doudou_blend/package.json", JSON.parse(readRepositoryFile("doudou_blend/package.json")).version],
  [
    "doudou_blend/src-tauri/tauri.conf.json",
    JSON.parse(readRepositoryFile("doudou_blend/src-tauri/tauri.conf.json")).version,
  ],
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
const tauriLibrarySource = readRepositoryFile("doudou_blend/src-tauri/src/lib.rs");
const backendSource = readRepositoryFile("doudou_blend/src/backend.ts");
if (!/pub\s+fn\s+master_json\s*\(/.test(coreLibrarySource)) {
  recordFailure("blend_kit 未提供统一 master_json() 入口");
}
if (!/blend_kit::master_json\s*\(/.test(wasmLibrarySource)) {
  recordFailure("WASM 未复用 blend_kit::master_json()");
}
if (!/fn\s+get_master_json\s*\(/.test(tauriLibrarySource)) {
  recordFailure("Tauri 未提供 get_master_json command");
}
if (!/invoke<string>\(\s*["']get_master_json["']\s*\)/.test(backendSource)) {
  recordFailure("Tauri 前端未通过 IPC 读取统一 Master");
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
    "doudou_blend/src-tauri/Cargo.toml",
    readCargoPackageLicense("doudou_blend/src-tauri/Cargo.toml"),
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
