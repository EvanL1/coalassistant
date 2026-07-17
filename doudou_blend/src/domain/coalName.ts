/** 煤名归一化: 全角空格转半角、去首尾空格、大小写无关。 */
export function normalizeCoalName(value: string): string {
  return value.replace(/　/g, " ").trim().toLowerCase();
}
