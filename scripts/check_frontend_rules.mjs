/**
 * 前端分层规则检查 (doudou_blend/src, 测试文件除外).
 *
 * 这些规则把"改动该落在哪一层"变成硬约束, 防止重构后又慢慢散开:
 *   1. fetch 收口: 只有 http.ts / auth.ts 能直接调 fetch.
 *   2. 界面层不碰传输细节: screens/** 与所有 *.tsx 不得导入 apiFetch / readOk / apiJson*.
 *   3. Web 存储收口: localStorage / sessionStorage 只许列出的几个数据层文件碰.
 *   4. 询价不写偏好: QuoteScreen 与 quoteDraft 不得从 storage / penaltyStorage 导入写函数.
 *   5. 事件名收口: "doudou:*" 事件名字面量只许写在 events.ts.
 *   6. 旧价格键: fob_override / frt_override / fob_history / is_price_anchor 只许 storage.ts 读取剔除.
 *
 * 注释不算违规 (先剥注释再查; 剥注释是轻量状态机, 不处理正则字面量里的 // —— 现有代码没有,
 * 自检会盯住我们依赖的行为). 存储键 "doudou_blend.xxx.v1" 不是事件名 (事件名形如 "doudou:...").
 * 脚本自带自检: 对故意违规的内联样例, 每条规则都必须报出来, 报不出来就是规则本身坏了.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const frontendRoot = join(repositoryRoot, "doudou_blend");
const failures = [];

/** 剥掉注释 (字符串保留): 行注释、块注释、单双引号、模板串 (模板串内的 ${} 不再深入). */
function stripComments(source) {
  let output = "";
  let state = "code";
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];
    if (state === "code") {
      if (char === "/" && next === "/") state = "line";
      else if (char === "/" && next === "*") { state = "block"; index += 1; }
      else if (char === "'") { state = "single"; output += char; }
      else if (char === '"') { state = "double"; output += char; }
      else if (char === "`") { state = "template"; output += char; }
      else output += char;
    } else if (state === "line") {
      if (char === "\n") { state = "code"; output += char; }
    } else if (state === "block") {
      if (char === "*" && next === "/") { state = "code"; index += 1; }
      else if (char === "\n") output += char;
    } else {
      // 字符串 / 模板串: 处理转义与收尾引号, 内容原样保留 (字符串内容不算注释).
      output += char;
      if (char === "\\") { output += next ?? ""; index += 1; continue; }
      if (state === "single" && char === "'") state = "code";
      else if (state === "double" && char === '"') state = "code";
      else if (state === "template" && char === "`") state = "code";
    }
  }
  return output;
}

function lineOf(source, offset) {
  return source.slice(0, offset).split("\n").length;
}

/** 模块路径去掉 .ts / .tsx 后缀 (tsconfig 允许带后缀导入, 两种写法指的是同一个模块). */
function normalizeModule(specifier) {
  return specifier.replace(/\.tsx?$/, "");
}

/**
 * 从 import 语句里抽 (模块路径, 导入名列表).
 * `import * as x` 与动态 `import("...")` 拿到的是整个模块, 逐名规则管不住, 标成 namespace 由规则一律按违规处理.
 */
function parseImports(source) {
  const imports = [];
  for (const match of source.matchAll(/import\s+(type\s+)?\{([^}]*)\}\s*from\s*["']([^"']+)["']/g)) {
    const wholeIsType = Boolean(match[1]);
    const names = match[2]
      .split(",")
      .map((part) => part.trim())
      .filter(Boolean)
      .map((part) => ({
        name: part.replace(/^type\s+/, "").split(/\s+as\s+/)[0].trim(),
        isType: wholeIsType || /^type\s+/.test(part),
      }));
    imports.push({ module: normalizeModule(match[3]), names, namespace: false, offset: match.index });
  }
  for (const match of source.matchAll(/import\s+\*\s+as\s+[\w$]+\s+from\s*["']([^"']+)["']/g)) {
    imports.push({ module: normalizeModule(match[1]), names: [], namespace: true, offset: match.index });
  }
  for (const match of source.matchAll(/(?<![\w$.])import\s*\(\s*["']([^"']+)["']\s*\)/g)) {
    imports.push({ module: normalizeModule(match[1]), names: [], namespace: true, offset: match.index });
  }
  return imports;
}

const FETCH_ALLOWED = new Set([
  "doudou_blend/src/http.ts",
  "doudou_blend/src/auth.ts",
]);
const STORAGE_ALLOWED = new Set([
  "doudou_blend/src/storage.ts",
  "doudou_blend/src/cloudStorage.ts",
  "doudou_blend/src/penaltyStorage.ts",
  "doudou_blend/src/localAccount.ts",
  "doudou_blend/src/domain/quoteDraft.ts",
]);
const QUOTE_FILES = new Set([
  "doudou_blend/src/screens/QuoteScreen.tsx",
  "doudou_blend/src/domain/quoteDraft.ts",
]);
const EVENTS_FILE = "doudou_blend/src/events.ts";
const LEGACY_PRICE_KEYS = /\b(?:fob_override|frt_override|fob_history|is_price_anchor)\b/g;

const rules = [
  {
    name: "fetch 收口",
    check(path, source) {
      if (FETCH_ALLOWED.has(path)) return [];
      const stripped = stripComments(source);
      return [...stripped.matchAll(/(?:(?<![\w$.])|(?<![\w$.])(?:window|globalThis|self)\.)fetch\s*\(/g)].map(
        (match) =>
          `${path}:${lineOf(stripped, match.index)} 直接调 fetch()。401 会话失效与同源 Cookie 只在 http.ts 的 apiFetch 里处理, ` +
          `绕过它请求就会漏掉过期跳转。改法: 用 http.ts 的 apiJson / apiJsonOrNull (确需 Response 原文才用 apiFetch); ` +
          `若是登录相关接口, 放 auth.ts。`,
      );
    },
  },
  {
    name: "界面层不碰传输细节",
    check(path, source) {
      if (!path.includes("/screens/") && !path.endsWith(".tsx")) return [];
      const violations = [];
      for (const entry of parseImports(source)) {
        if (!/(^|\/)http$/.test(entry.module)) continue;
        const banned = entry.namespace
          ? ["整个 http 模块 (import * / 动态 import)"]
          : entry.names.map((item) => item.name).filter((name) => /^(apiFetch|readOk|apiJson|apiJsonOrNull)$/.test(name));
        if (banned.length > 0) {
          violations.push(
            `${path}:${lineOf(source, entry.offset)} 界面层导入了 ${banned.join("/")}。` +
            `组件只该 import 数据层模块 (quotes.ts / admin.ts / coalPrices.ts …) 的函数, 由数据层走 http.ts; ` +
            `组件直接发请求会绕过统一的错误与 401 处理。改法: 在对应数据层模块加函数, 组件改调它 (errorMessage 等展示工具可以直接从 http.ts 导入)。`,
          );
        }
      }
      return violations;
    },
  },
  {
    name: "Web 存储收口",
    check(path, source) {
      if (STORAGE_ALLOWED.has(path)) return [];
      const stripped = stripComments(source);
      return [...stripped.matchAll(/\b(?:localStorage|sessionStorage)\b/g)].map(
        (match) =>
          `${path}:${lineOf(stripped, match.index)} 碰了 Web 存储。本机缓存的读写只许在 ${[...STORAGE_ALLOWED].map((p) => p.split("/").pop()).join(" / ")} 里, ` +
          `散开读写会绕开云端同步 (cloudStorage 的基线/冲突判断)。改法: 在 storage.ts (或对应存储模块) 加读写函数再调用。`,
      );
    },
  },
  {
    name: "询价不写偏好",
    check(path, source) {
      if (!QUOTE_FILES.has(path)) return [];
      const violations = [];
      for (const entry of parseImports(source)) {
        if (!/(^|\/)(penaltyStorage|storage)$/.test(entry.module)) continue;
        const banned = entry.namespace
          ? ["整个存储模块 (import * / 动态 import)"]
          : entry.names.filter((item) => !item.isType && !/^get[A-Z]/.test(item.name)).map((item) => item.name);
        if (banned.length > 0) {
          violations.push(
            `${path}:${lineOf(source, entry.offset)} 询价流程从存储模块导入了 ${banned.join("/")} (只许导入 get* 读函数与类型)。` +
            `询价精算只用当场填的价与指标求解, 不进煤池、不写本地或云端偏好、不跨次沿用; 写了就破这个规矩。` +
            `改法: 状态留在询价页自己的草稿 (quoteDraft) 里, 不要落偏好存储。`,
          );
        }
      }
      return violations;
    },
  },
  {
    name: "事件名收口",
    check(path, source) {
      if (path === EVENTS_FILE) return [];
      const stripped = stripComments(source);
      return [...stripped.matchAll(/["'`](doudou:[^"'`]*)["'`]/g)].map(
        (match) =>
          `${path}:${lineOf(stripped, match.index)} 写了事件名字面量 "${match[1]}"。` +
          `跨模块通知的事件名全部集中在 events.ts (APP_EVENTS / USER_STORAGE_EVENTS), 各写各的字面量会对不上、收不到。` +
          `改法: 在 events.ts 登记事件名, 这里用 emit(...) / listen(...) 并引用常量。`,
      );
    },
  },
  {
    name: "旧价格键只许 storage.ts 剔除",
    check(path, source) {
      if (path === "doudou_blend/src/storage.ts") return [];
      const stripped = stripComments(source);
      return [...stripped.matchAll(LEGACY_PRICE_KEYS)].map(
        (match) =>
          `${path}:${lineOf(stripped, match.index)} 读写了旧偏好价格键 "${match[0]}"。` +
          `煤价一律按质量估算, 这些键只在 storage.ts 读取时剔除, 别处读写就会把废口径又接回来。` +
          `改法: 删掉这段读写; 要估价走 coalPrices.ts / quotes.ts。`,
      );
    },
  },
];

// ---------------------------------------------------------------------------
// 自检: 每条规则对故意违规的样例必须报出, 对合法样例必须放行.
// 规则改松了 (样例报不出) 或剥注释坏了 (合法样例误报) 都算脚本本身失败.
// ---------------------------------------------------------------------------
const SELF_CHECK = [
  { rule: "fetch 收口", path: "doudou_blend/src/foo.ts", source: 'const r = await fetch("/api/x");', violations: 1 },
  { rule: "fetch 收口", path: "doudou_blend/src/http.ts", source: 'const r = await fetch("/api/x");', violations: 0 },
  { rule: "fetch 收口", path: "doudou_blend/src/foo.ts", source: 'await apiFetch("/api/x"); // fetch("/x") 在注释里不算', violations: 0 },
  { rule: "fetch 收口", path: "doudou_blend/src/foo.ts", source: 'await window.fetch("/api/x"); await globalThis.fetch("/api/y");', violations: 2 },
  { rule: "fetch 收口", path: "doudou_blend/src/foo.ts", source: "const rows = await repository.fetch(id); // 自定义方法不算", violations: 0 },
  {
    rule: "界面层不碰传输细节",
    path: "doudou_blend/src/screens/Foo.tsx",
    source: 'import { apiJson } from "../http";',
    violations: 1,
  },
  {
    rule: "界面层不碰传输细节",
    path: "doudou_blend/src/screens/Foo.tsx",
    source: 'import {\n  errorMessage,\n  apiFetch,\n} from "../http.ts";',
    violations: 1,
  },
  {
    rule: "界面层不碰传输细节",
    path: "doudou_blend/src/screens/Foo.tsx",
    source: 'import * as http from "../http";\nconst lazy = await import("../http");',
    violations: 2,
  },
  {
    rule: "界面层不碰传输细节",
    path: "doudou_blend/src/screens/Foo.tsx",
    source: 'import { errorMessage } from "../http";',
    violations: 0,
  },
  {
    rule: "界面层不碰传输细节",
    path: "doudou_blend/src/admin.ts",
    source: 'import { apiJson } from "./http";',
    violations: 0,
  },
  { rule: "Web 存储收口", path: "doudou_blend/src/foo.ts", source: "localStorage.setItem('k', 'v');", violations: 1 },
  { rule: "Web 存储收口", path: "doudou_blend/src/storage.ts", source: "localStorage.setItem('k', 'v');", violations: 0 },
  { rule: "Web 存储收口", path: "doudou_blend/src/foo.ts", source: "// 只存 localStorage, 不进云\n/* sessionStorage 也是 */", violations: 0 },
  {
    rule: "询价不写偏好",
    path: "doudou_blend/src/screens/QuoteScreen.tsx",
    source: 'import { setQuantity } from "../storage";',
    violations: 1,
  },
  {
    rule: "询价不写偏好",
    path: "doudou_blend/src/screens/QuoteScreen.tsx",
    source: 'import { getQuantity } from "../storage";\nimport { getPenaltyTemplate } from "../penaltyStorage";',
    violations: 0,
  },
  {
    rule: "询价不写偏好",
    path: "doudou_blend/src/screens/QuoteScreen.tsx",
    source: 'import { enableAllCoals, replaceUserStorageSnapshot } from "../storage.ts";',
    violations: 1,
  },
  {
    rule: "询价不写偏好",
    path: "doudou_blend/src/domain/quoteDraft.ts",
    source: 'import * as storage from "../storage";',
    violations: 1,
  },
  {
    rule: "询价不写偏好",
    path: "doudou_blend/src/screens/QuoteScreen.tsx",
    source: 'import { getQuantity, type CoalPref } from "../storage";\nimport type { PenaltyTemplate } from "../penaltyStorage";',
    violations: 0,
  },
  {
    rule: "询价不写偏好",
    path: "doudou_blend/src/screens/TodayScreen.tsx",
    source: 'import { setQuantity } from "../storage";',
    violations: 0,
  },
  { rule: "事件名收口", path: "doudou_blend/src/foo.ts", source: 'dispatchEvent(new CustomEvent("doudou:coals_changed"));', violations: 1 },
  { rule: "事件名收口", path: "doudou_blend/src/events.ts", source: 'return `doudou:${name}`;', violations: 0 },
  { rule: "事件名收口", path: "doudou_blend/src/foo.ts", source: 'const KEY = "doudou_blend.coal_prefs.v1"; // 存储键不是事件名', violations: 0 },
  { rule: "旧价格键只许 storage.ts 剔除", path: "doudou_blend/src/foo.ts", source: 'delete prefs[name].fob_override;', violations: 1 },
  { rule: "旧价格键只许 storage.ts 剔除", path: "doudou_blend/src/storage.ts", source: 'delete prefs[name].fob_override;', violations: 0 },
];

function runSelfCheck() {
  for (const testCase of SELF_CHECK) {
    const rule = rules.find((candidate) => candidate.name === testCase.rule);
    const found = rule.check(testCase.path, testCase.source).length;
    if (found !== testCase.violations) {
      failures.push(
        `自检失败 [${testCase.rule}]: 样例 ${JSON.stringify(testCase.source.slice(0, 60))} ` +
        `(${testCase.path}) 期望报 ${testCase.violations} 条, 实际报 ${found} 条`,
      );
    }
  }
}

function collectFrontendSources(directory) {
  const collected = [];
  for (const entryName of readdirSync(directory)) {
    if (entryName === "node_modules" || entryName === "dist") continue;
    const absolutePath = join(directory, entryName);
    if (statSync(absolutePath).isDirectory()) {
      collected.push(...collectFrontendSources(absolutePath));
    } else if ([".ts", ".tsx"].includes(extname(entryName))) {
      collected.push(absolutePath);
    }
  }
  return collected;
}

runSelfCheck();

let checkedFiles = 0;
for (const absolutePath of collectFrontendSources(join(frontendRoot, "src"))) {
  const relativePath = absolutePath.slice(repositoryRoot.length + 1);
  // 测试文件与测试帮手 (src/test/) 不受分层规则约束.
  if (/\.test\.tsx?$/.test(absolutePath) || relativePath.includes("/src/test/")) continue;
  checkedFiles += 1;
  const source = readFileSync(absolutePath, "utf8");
  for (const rule of rules) {
    failures.push(...rule.check(relativePath, source));
  }
}

if (failures.length > 0) {
  console.error(`前端规则检查失败 (${failures.length}):`);
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log(`前端规则检查通过: ${rules.length} 条规则, ${checkedFiles} 个生产文件, 含 ${SELF_CHECK.length} 项自检`);
