/**
 * 手机上敲进来的数字: 全角转半角; 价格类认千分位 (2,050 = 2050); 指标类把单个逗号当小数点
 * (有的输入法小数点是逗号, 65,3 = 65.3). 认不出来就报一句说得清的错, 绝不悄悄吞字符.
 */

export type NumberKind = "price" | "indicator";

export type ParsedNumber =
  | { ok: true; value: number | null }
  | { ok: false; error: string };

const EXAMPLE: Record<NumberKind, string> = { price: "2050", indicator: "65.3" };

/** 全角数字与符号转半角, 全角空格当空格. */
export function toHalfWidth(raw: string): string {
  return raw
    .replace(/[\uFF10-\uFF19\uFF0B\uFF0C\uFF0D\uFF0E]/g, (ch) =>
      String.fromCharCode(ch.charCodeAt(0) - 0xfee0),
    )
    .replace(/\u3000/g, " ");
}

/** 去首尾空白、全角转半角、按字段类型处理逗号; 返回可直接交给 Number 的文本, 认不出返回 null.
 *  只去首尾: 数字中间夹的空白多半是黏贴带进来的分隔, "12 5" 拼成 125 就悄悄改了数. */
function canonical(raw: string, kind: NumberKind): string | null {
  const text = toHalfWidth(raw).trim();
  if (/\s/.test(text)) return null;
  let normalized = text;
  if (normalized.includes(",")) {
    if (kind === "price") {
      if (!/^[+-]?\d{1,3}(,\d{3})+(\.\d+)?$/.test(normalized)) return null;
      normalized = normalized.replace(/,/g, "");
    } else {
      if (!/^[+-]?\d*,\d+$/.test(normalized)) return null;
      normalized = normalized.replace(",", ".");
    }
  }
  return /^[+-]?(\d+\.?\d*|\.\d+)$/.test(normalized) ? normalized : null;
}

/**
 * 输到一半的中间态: 末尾是小数点/逗号, 只有正负号, 或千分位写到一半 ("2,0"、"2,05" 正在
 * 敲成 "2,050"). 输入过程中不当错误提示, 失焦或保存时才报.
 */
export function isPartialNumberInput(raw: string): boolean {
  const text = toHalfWidth(raw).trim();
  if (text === "") return false;
  return (
    /^[+-]$/.test(text) ||
    /[.,]$/.test(text) ||
    /^[+-]?\d{1,3}(,\d{3})*,\d{1,2}$/.test(text)
  );
}

/** 空 (含只有空白) = value null; 其余要么是有限数, 要么给出错误提示. */
export function parseNumberInput(raw: string, kind: NumberKind): ParsedNumber {
  if (raw.trim() === "" || toHalfWidth(raw).trim() === "") return { ok: true, value: null };
  const text = canonical(raw, kind);
  const value = text == null ? Number.NaN : Number(text);
  if (!Number.isFinite(value)) {
    return { ok: false, error: `请输入数字, 例如 ${EXAMPLE[kind]}` };
  }
  return { ok: true, value };
}

/** 能认出的数; 空或认不出都是 null (只给"不合法已另行拦住"的地方用). */
export function numberOrNull(raw: string, kind: NumberKind): number | null {
  const parsed = parseNumberInput(raw, kind);
  return parsed.ok ? parsed.value : null;
}

/** 已填但认不出时的错误提示; 空或合法返回 null. */
export function numberInputError(raw: string, kind: NumberKind): string | null {
  const parsed = parseNumberInput(raw, kind);
  return parsed.ok ? null : parsed.error;
}
