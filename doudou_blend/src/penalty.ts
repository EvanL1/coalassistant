import { INDICATOR_LABEL } from "./types";
import type {
  Direction,
  Penalty,
  PenaltyTier,
  PurchaseClause,
  PurchaseTerms,
} from "./types";

/** 模板里的一条条款: 只有条款本身, 保证值由每种煤各自提供. */
export interface PenaltyTemplateClause {
  indicator: string;
  direction: Direction;
  penalty: Penalty;
}

/** 全局扣款模板: 行业通用条款, 所有煤默认套用. */
export interface PenaltyTemplate {
  clauses: PenaltyTemplateClause[];
  contract_moisture?: number | null;
  moisture_excess_double_threshold?: number | null;
}

/** 单煤覆盖: 只列出与模板不同的条款. */
export interface CoalPenaltyOverride {
  clauses?: PenaltyTemplateClause[];
  /**
   * 该煤明确不适用的模板条款指标: 即使有保证值也不产出条款, 且不计入孤儿保证值
   * (这是用户主动排除, 不是数据丢失). 真实采购合同确实会只对部分指标计价.
   */
  excluded_indicators?: string[];
  contract_moisture?: number | null;
  moisture_excess_double_threshold?: number | null;
}

/**
 * 合并结果. `terms` 供 core 使用; `orphanedGuarantees` 标出"有保证值却找不到
 * 条款"的指标 —— 调用方应据此提示用户, 而不是静默按无扣款处理.
 *
 * 孤儿保证值最常见的成因: 全局模板只存 localStorage 不跨设备同步
 * (见 penaltyStorage.ts), 换设备登录后 CoalPref 里的 purchase_guarantees
 * 随 coal_prefs 正常同步过来了, 但模板没有 —— 这种煤本该计价却悄悄零扣款,
 * 两台设备算出的成本会不一致但界面上什么都不会报错。
 */
export interface MergedPurchaseTerms {
  terms: PurchaseTerms | null;
  orphanedGuarantees: string[];
}

/**
 * 把合同原文的"每 step 个单位扣 amount 元"换算成 元/吨·单位.
 * 合同写"每超 0.1% 扣 8 元/吨" ⇒ tierRate({ step: 0.1, amount: 8 }) = 80.
 * 让用户照抄合同, 不做心算 —— 8 与 80 差一个量级。
 *
 * 用具名对象参数, 不用位置参数: `tierRate(8, 0.1)` 这种参数顺序传反的调用,
 * 位置参数会静默算出一个看似合理但错误的数字, 对象参数会直接类型对不上/
 * 值域检查失败, 编译期或运行期都能截住。
 *
 * 非法输入(step/amount 非有限数、step<=0、amount<0)返回 null, 不是 0 ——
 * 0 本身是合法的零费率档位(某档不扣钱), 不能拿它当错误信号, 调用方必须
 * 显式处理 null。
 */
export function tierRate({
  step,
  amount,
}: {
  step: number;
  amount: number;
}): number | null {
  if (!Number.isFinite(step) || step <= 0) return null;
  if (!Number.isFinite(amount) || amount < 0) return null;
  const rate = amount / step;
  // 两个各自合法的数也能除出 Infinity (1e300 / 1e-320). core 的 is_finite()
  // 会拒掉它, 前端不能先放行 —— 这是整个功能算钱的那一步.
  return Number.isFinite(rate) ? rate : null;
}

/**
 * 合并全局模板与单煤覆盖, 配上该煤的保证值, 产出 core 需要的 PurchaseTerms。
 *
 * 规则:
 *   - 覆盖按指标逐条替换模板条款(不是整体替换); `excluded_indicators` 里的
 *     指标即使在模板或覆盖里有条款也不产出(实现"单煤覆盖压制模板条款",
 *     而不仅是新增/替换)。
 *   - 没有保证值的指标不产出条款(没有保证值就无从判定偏离), 也不算孤儿
 *     (这只是"没配置", 不是"配置丢了")。
 *   - 有保证值却在模板+覆盖里都找不到条款的指标(且未被显式排除), 计入
 *     `orphanedGuarantees` —— 调用方应提示用户, 而不是当作用户没配置。
 *   - `contract_moisture` / `moisture_excess_double_threshold`: 覆盖对象里
 *     这个键为具体数字就用它; 为 `null` 是显式关闭, 不回退模板; 键不存在
 *     或值为 `undefined` 都视为"没设置", 回退模板。
 *     "键不存在"与"值为 undefined"必须同等对待(而不是像早期版本那样用
 *     `in` 单独区分), 因为 `JSON.stringify` 会丢弃值为 undefined 的键 ——
 *     区分它们会让同一个覆盖对象在写入/读出 localStorage 前后表现不同,
 *     用户能看到的症状是"配方价格随页面刷新变化"。
 *   - `terms === null` 表示该煤没有任何可用采购条款, 调用方应省略
 *     `purchase_terms` 字段。
 */
export function mergePurchaseTerms(
  template: PenaltyTemplate | null,
  override: CoalPenaltyOverride | undefined,
  guarantees: Partial<Record<string, number>>,
): MergedPurchaseTerms {
  const byIndicator = new Map<string, PenaltyTemplateClause>();
  for (const clause of template?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }
  for (const clause of override?.clauses ?? []) {
    byIndicator.set(clause.indicator, clause);
  }
  const excluded = new Set(override?.excluded_indicators ?? []);

  const clauses: PurchaseClause[] = [];
  const orphanedGuarantees: string[] = [];
  for (const [indicator, guarantee] of Object.entries(guarantees)) {
    if (typeof guarantee !== "number" || !Number.isFinite(guarantee)) continue;
    if (excluded.has(indicator)) continue;
    const clause = byIndicator.get(indicator);
    if (!clause) {
      orphanedGuarantees.push(indicator);
      continue;
    }
    clauses.push({
      indicator,
      direction: clause.direction,
      guarantee,
      penalty: clause.penalty,
    });
  }

  // 没有任何条款落地 = 该煤未配置采购扣款, 即使模板/覆盖带了合同水分也不该
  // 套用(水分双倍计入是"该煤有采购合同"的推论, 不该在没有条款时生效)。
  if (clauses.length === 0) {
    return { terms: null, orphanedGuarantees };
  }

  const contract_moisture = inheritableMoistureField(
    override?.contract_moisture,
    template?.contract_moisture,
  );
  const moisture_excess_double_threshold = inheritableMoistureField(
    override?.moisture_excess_double_threshold,
    template?.moisture_excess_double_threshold,
  );

  return {
    terms: { clauses, contract_moisture, moisture_excess_double_threshold },
    orphanedGuarantees,
  };
}

/**
 * `contract_moisture` / `moisture_excess_double_threshold` 共用的继承规则:
 * 覆盖值是具体数字就用它; 是 `null` 就显式关闭(不回退模板); 是 `undefined`
 * (无论键是否存在, 两者在 JS 里读取结果相同)就当没设置, 回退模板的值。
 */
function inheritableMoistureField(
  overrideValue: number | null | undefined,
  templateValue: number | null | undefined,
): number | null {
  return overrideValue !== undefined ? overrideValue : (templateValue ?? null);
}

/**
 * 档位录入原文: 用户照抄合同的"每 step 个单位扣 amount 元/吨", 由前端换算成
 * core 要的 rate。两栏分开录入是刻意的 —— 合同写"每超 0.1% 扣 8 元/吨",
 * 让用户自己心算成 80 的话, 填成 8 不会有任何东西看起来是坏的, 每吨扣款却差
 * 一个量级。
 */
export interface TierDraft {
  /** 合同原文的"每 ___"(个指标单位). */
  step: string;
  /** 合同原文的"扣 ___ 元/吨". */
  amount: string;
  /** 本档覆盖多少偏离; 末档留空(末档吃掉剩下的全部超出). */
  width: string;
}

/** 一条计价条款的录入态. 界面上的真源是它, `Penalty` 永远由它换算得出. */
export interface PenaltyDraft {
  tiers: TierDraft[];
  reject: string;
}

/** 换算结果: `penalty` 与 `error` 恰有一个非空. */
export interface PenaltyDraftResult {
  penalty: Penalty | null;
  error: string | null;
}

/** 单煤保证值的录入提示. `error` 会让 core 报错, `hint` 只是没配全. */
export interface GuaranteeIssue {
  level: "error" | "hint";
  message: string;
}

/**
 * 偏离量在这个方向上该叫什么: 上限型是"超出", 下限型是"不足"。
 *
 * 界面文案与报错文案共用这一处 —— 下限型指标(粘结 ≥75)差 5 点是不够, 不是
 * 超标, 两个方向都写成"超出", 标签就在描述一个它不是的东西。
 */
export function deviationNoun(direction: Direction): string {
  return direction === "Lower" ? "不足" : "超出";
}

/** 判定说法: 上限型"超标", 下限型"不达标". 与 [`deviationNoun`] 同一处定义. */
export function offSpecWord(direction: Direction): string {
  return direction === "Lower" ? "不达标" : "超标";
}

/** 越过拒收线的说法: 上限型"超过", 下限型"低于". */
export function rejectCrossWord(direction: Direction): string {
  return direction === "Lower" ? "低于" : "超过";
}

/**
 * 指标在合同里的计量单位。
 *
 * 单位标错与"该填 80 却填了 8"是同一类事故: 用户照抄合同, 抄得再忠实也是错的,
 * 而且界面上没有任何东西看起来是坏的。所以八项逐一列全, 不靠"其余按 %"兜底
 * —— 兜底正是 Y(mm) 和 petro(无量纲) 被标成 % 的原因。
 *
 * 数据来源是 `blend_kit_rs/data/coal_master.json` 的字段说明:
 * "Y": "胶质层最大厚度 mm", "petro": "岩相 (反射率分布度量)"。
 */
const INDICATOR_UNIT: Record<string, string> = {
  S: "%",
  A: "%",
  V: "%",
  M: "%",
  // 粘结指数 G 与焦炭强度 CSR 是无量纲的, 配煤师论"点".
  G: "点",
  CSR: "点",
  Y: "mm",
  // 岩相是反射率的分布度量, 没有单位 —— 空字符串, 宁可不写也不能写错.
  petro: "",
};

export function indicatorUnit(indicator: string): string {
  return INDICATOR_UNIT[indicator] ?? "";
}

/** 空白视作 NaN: `Number("")` 是 0, 会把"没填"错当成"填了 0". */
function draftNumber(raw: string): number {
  return raw.trim() === "" ? Number.NaN : Number(raw);
}

/** 去掉浮点噪声再转字符串, 免得错误提示里出现 59.999999999999996. */
function formatNumber(value: number): string {
  return String(Number(value.toPrecision(12)));
}

/**
 * 拒收线与边界的关系检查, 卖出侧(边界=合同上/下限)与买入侧(边界=保证值)共用。
 * 两侧规则在 core 里本来就是同一个 `validate_penalty`, 前端也只留一份实现 ——
 * 抄成两份, 改了一侧忘了另一侧, 用户就会在一个屏幕上通过、另一个屏幕上失败。
 */
function rejectBoundError(
  direction: Direction,
  bound: number,
  reject: number,
  boundLabel: string,
): string | null {
  if (direction === "Upper" && reject < bound) {
    return `拒收线不能低于${boundLabel} ${formatNumber(bound)}: 拒收线是"超到多少就整批不收", 必须在${boundLabel}之外`;
  }
  if (direction === "Lower" && reject > bound) {
    return `拒收线不能高于${boundLabel} ${formatNumber(bound)}: 拒收线是"低到多少就整批不收", 必须在${boundLabel}之外`;
  }
  return null;
}

/** 新开一条计价条款时的空白录入态: 只预填合同最常见的"每 0.1", 其余留给用户. */
export function emptyPenaltyDraft(): PenaltyDraft {
  return { tiers: [{ step: "0.1", amount: "", width: "" }], reject: "" };
}

/**
 * 把已存的条款回显成录入态。
 *
 * 一律回显成"每 1 个单位扣 rate 元": 用户当初照抄的"每 0.1%"没有存下来
 * (`Penalty` 只留换算后的 rate, 那是 core 的 schema), 猜一个 0.1 回去要做除法,
 * 会引入浮点噪声; 每 1 个单位是恒等回显, 数值上与用户录的完全等价。
 */
export function penaltyToDraft(penalty: Penalty): PenaltyDraft {
  return {
    tiers: penalty.tiers.map((tier) => ({
      step: "1",
      amount: formatNumber(tier.rate),
      width: tier.width != null ? formatNumber(tier.width) : "",
    })),
    reject: formatNumber(penalty.reject),
  };
}

/**
 * 把录入原文换算并校验成 core 的 `Penalty`, 规则与 Rust `quality::validate_penalty`
 * 一致 —— 提前在字段旁报错, 而不是等求解失败才告诉用户。
 *
 * `bound` 为 null 表示"这里没有可比的边界"(采购扣款模板的保证值逐煤不同),
 * 此时跳过拒收线比对, 其余规则照查; 该比对改由录入保证值的地方
 * ([`guaranteeIssue`]) 完成。
 */
export function penaltyFromDraft(
  draft: PenaltyDraft,
  options: {
    indicator: string;
    direction: Direction;
    /** 卖出侧= 合同上/下限, 买入侧= 保证值; null = 此处无从比对. */
    bound: number | null;
    /** 错误提示里怎么称呼这条边界, 必须与界面上那个输入框的标签一致. */
    boundLabel: string;
  },
): PenaltyDraftResult {
  const fail = (error: string): PenaltyDraftResult => ({ penalty: null, error });
  const unit = indicatorUnit(options.indicator);
  const noun = deviationNoun(options.direction);

  if (options.direction === "Range") {
    return fail("区间型指标不能按计价扣款: 改成只卡上限或只卡下限才能计价");
  }
  if (draft.tiers.length === 0) {
    return fail("至少要填一档扣款");
  }

  const tiers: PenaltyTier[] = [];
  let previousRate = Number.NEGATIVE_INFINITY;
  for (const [index, tier] of draft.tiers.entries()) {
    const position = index + 1;
    const step = draftNumber(tier.step);
    const amount = draftNumber(tier.amount);
    // 合法与否只由 tierRate 说了算(0 元是合法档位, 不能拿 0 当错误信号);
    // 下面两条分支只负责把"哪一栏填坏了"讲清楚, 不重复判定。
    const rate = tierRate({ step, amount });
    if (rate == null) {
      return fail(
        !Number.isFinite(step) || step <= 0
          ? `第 ${position} 档「每」要填一个大于 0 的数: 合同写"每超 0.1${unit}"就填 0.1`
          : `第 ${position} 档「扣」要填 0 或更大的数: 合同写"扣 8 元/吨"就填 8`,
      );
    }
    if (rate <= previousRate) {
      return fail(
        `第 ${position} 档要比上一档扣得更狠: 折成每 1${unit}, 本档 ${formatNumber(rate)} 元/吨, 上一档 ${formatNumber(previousRate)} 元/吨`,
      );
    }
    previousRate = rate;

    const isLast = index === draft.tiers.length - 1;
    if (isLast) {
      if (tier.width.trim() !== "") {
        return fail(`末档不能填「本档覆盖」: 最后一档吃掉剩下的全部${noun}`);
      }
      tiers.push({ rate });
    } else {
      const width = draftNumber(tier.width);
      if (!Number.isFinite(width) || width <= 0) {
        return fail(
          // 不往这句里嵌单位: 岩相没有单位("这一档管超出的前几" 读不通),
          // 而输入框旁边本来就标着单位.
          `第 ${position} 档要填「本档覆盖」: 这一档管${noun}的前一段, 填满才进下一档`,
        );
      }
      tiers.push({ rate, width });
    }
  }

  const reject = draftNumber(draft.reject);
  if (!Number.isFinite(reject)) {
    return fail("拒收线要填一个数: 超过它整批不收, 不再按扣款算");
  }
  if (options.bound != null) {
    const boundError = rejectBoundError(
      options.direction,
      options.bound,
      reject,
      options.boundLabel,
    );
    if (boundError) return fail(boundError);
  }

  return { penalty: { tiers, reject }, error: null };
}

/**
 * 单煤保证值录入时的即时校验。
 *
 * 走 [`mergePurchaseTerms`] 而不是自己翻模板: 哪条条款对这个煤生效(模板/覆盖/
 * 排除)的规则只有那一处, 这里照用, 就不会出现"录入时说没问题、求解时报错"。
 */
export function guaranteeIssue(args: {
  template: PenaltyTemplate | null;
  override: CoalPenaltyOverride | undefined;
  indicator: string;
  guarantee: number;
}): GuaranteeIssue | null {
  const { template, override, indicator, guarantee } = args;
  const label = INDICATOR_LABEL[indicator] ?? indicator;
  if (!Number.isFinite(guarantee)) {
    return { level: "error", message: "保证值要填一个数" };
  }

  const { terms, orphanedGuarantees } = mergePurchaseTerms(template, override, {
    [indicator]: guarantee,
  });
  if (orphanedGuarantees.includes(indicator)) {
    return {
      level: "hint",
      message: `还没有${label}的采购扣款条款: 去煤池顶部的「采购扣款模板」加一条, 否则这个保证值不影响成本`,
    };
  }
  const clause = terms?.clauses.find((c) => c.indicator === indicator);
  if (!clause) return null; // 该煤明确排除了这项, 不是漏配

  if (indicator === "M" && terms?.contract_moisture != null) {
    return {
      level: "error",
      message: `水分已经按扣量计(合同水分 ${formatNumber(terms.contract_moisture)}%), 不能再按扣价扣: 两种算法只能留一种`,
    };
  }

  const boundError = rejectBoundError(
    clause.direction,
    guarantee,
    clause.penalty.reject,
    "保证值",
  );
  return boundError ? { level: "error", message: boundError } : null;
}

/** 采购扣款模板里一条条款的录入态. 保证值不在这里 —— 它逐煤不同. */
export interface TemplateClauseDraft {
  indicator: string;
  /** core 只接受 Upper / Lower, 模板界面也就只给这两个选项. */
  direction: "Upper" | "Lower";
  penalty: PenaltyDraft;
}

/**
 * 模板换算结果。`error` 只报整份模板层面的问题(重复指标、水分范围、一条条款
 * 都没有), 单条条款自己的问题在 `clauseErrors[i]` 里, 与该条款一一对应 ——
 * 这样界面能把每条错显示在出错的那条下面, 又不必自己再算一遍。
 * `template` 非空 ⟺ `error` 为 null 且 `clauseErrors` 全为 null。
 */
export interface TemplateDraftResult {
  template: PenaltyTemplate | null;
  error: string | null;
  clauseErrors: (string | null)[];
}

export interface TemplateDraft {
  clauses: TemplateClauseDraft[];
  /** 合同水分 (%), 走扣量; 留空 = 不设. */
  contractMoisture: string;
  /** 超过它的水分按 2 倍计入; 留空 = 不设. */
  doubleThreshold: string;
}

/** 0~100 之间的可选百分数; 留空返回 null, 填坏返回 undefined. */
function optionalPercent(raw: string): number | null | undefined {
  if (raw.trim() === "") return null;
  const value = Number(raw);
  return Number.isFinite(value) && value >= 0 && value <= 100 ? value : undefined;
}

/**
 * 把采购扣款模板的录入态换算并校验成可存的模板, 规则对齐 Rust
 * `quality::validate_purchase_terms` 里模板这一侧能查的部分。
 *
 * 查不了的有两条, 都因为模板本身不含保证值:
 *   - 拒收线与保证值的关系;
 *   - 水分同时走扣量与扣价。
 * 两条都在录保证值的地方查 (见 [`guaranteeIssue`]) —— 在这里查会对"没填水分
 * 保证值的煤"误报, 而那些煤根本不会产出水分条款。
 */
export function templateFromDraft(draft: TemplateDraft): TemplateDraftResult {
  // 每条条款各自的错只在这里算一次, 调用方直接用 clauseErrors[i] 显示在那条
  // 条款下面 —— 界面再算一遍就是同一件事两处实现, 这个功能其余部分都在躲它.
  const converted = draft.clauses.map((clause) =>
    penaltyFromDraft(clause.penalty, {
      indicator: clause.indicator,
      direction: clause.direction,
      // 保证值逐煤不同, 模板这里没有可比的边界.
      bound: null,
      boundLabel: "保证值",
    }),
  );
  const clauseErrors = converted.map((result) => result.error);

  // 整份模板层面的错, 与条款自身的错各报各的: 掺在一起会让"灰有两条条款"
  // 被一个无关的档位错盖住, 改完那个才冒出来.
  const templateError = templateLevelError(draft);

  if (templateError != null || clauseErrors.some((error) => error != null)) {
    return { template: null, error: templateError, clauseErrors };
  }

  const clauses: PenaltyTemplateClause[] = draft.clauses.map((clause, index) => ({
    indicator: clause.indicator,
    direction: clause.direction,
    penalty: converted[index].penalty!,
  }));
  return {
    template: {
      clauses,
      contract_moisture: optionalPercent(draft.contractMoisture) as number | null,
      moisture_excess_double_threshold: optionalPercent(
        draft.doubleThreshold,
      ) as number | null,
    },
    error: null,
    clauseErrors,
  };
}

/** 整份模板层面的错(与单条条款无关的那些); null = 这一层没问题. */
function templateLevelError(draft: TemplateDraft): string | null {
  if (draft.clauses.length === 0) {
    return "至少要有一条扣款条款: 不想要模板了就按「清空模板」";
  }
  const seen = new Set<string>();
  for (const clause of draft.clauses) {
    if (seen.has(clause.indicator)) {
      const label = INDICATOR_LABEL[clause.indicator] ?? clause.indicator;
      return `${label}有两条条款: 同一项只能留一条, 否则这项的扣款会被算两遍`;
    }
    seen.add(clause.indicator);
  }
  if (optionalPercent(draft.contractMoisture) === undefined) {
    return "合同水分要填 0~100 之间的数";
  }
  if (optionalPercent(draft.doubleThreshold) === undefined) {
    return "水分双倍阈值要填 0~100 之间的数";
  }
  return null;
}

/** 把已存的模板回显成录入态. */
export function templateToDraft(template: PenaltyTemplate | null): TemplateDraft {
  return {
    clauses: (template?.clauses ?? []).map((clause) => ({
      indicator: clause.indicator,
      direction: clause.direction === "Lower" ? "Lower" : "Upper",
      penalty: penaltyToDraft(clause.penalty),
    })),
    contractMoisture:
      template?.contract_moisture != null ? formatNumber(template.contract_moisture) : "",
    doubleThreshold:
      template?.moisture_excess_double_threshold != null
        ? formatNumber(template.moisture_excess_double_threshold)
        : "",
  };
}
