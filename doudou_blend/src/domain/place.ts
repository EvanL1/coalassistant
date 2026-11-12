/**
 * 产地: 省 / 市 / 县三个字段, 取行政区划全称 (山西省 / 晋中市 / 灵石县), 只能从名录里选.
 * 估价对标按 省 → 市 逐级比 (核心 `pricing::place_penalty`); 县只记录与展示.
 *
 * 名录是 `src/data/divisions.json` (省 → 市 → 县), 用到时才加载; master 自检脚本
 * (`scripts/check_master_data.mjs`) 读的是同一份. 来源: github.com/modood/Administrative-divisions-of-China
 * 的 dist/pca.json (2025-12-27 版, WTFPL 许可), 数据取自国家统计局的统计用区划 —— 所以
 * 县一级里混有开发区、示范区; 直辖市的"市"一级是 "市辖区" / "县" 两项. 要更新就整份换掉.
 */
export interface Place {
  province?: string | null;
  city?: string | null;
  county?: string | null;
}

/** 省 → 市 → 县. 直辖市的"市"一级是 "市辖区" / "县". */
export type Divisions = Record<string, Record<string, string[]>>;

let cache: Promise<Divisions> | null = null;

export function loadDivisions(): Promise<Divisions> {
  cache ??= import("../data/divisions.json")
    .then((module) => module.default as Divisions)
    .catch((error: unknown) => {
      // 加载失败不记住: 下次打开选择框再试.
      cache = null;
      throw error;
    });
  return cache;
}

export function provincesOf(divisions: Divisions): string[] {
  return Object.keys(divisions);
}

export function citiesOf(divisions: Divisions, province: string | null | undefined): string[] {
  return province ? Object.keys(divisions[province] ?? {}) : [];
}

export function countiesOf(
  divisions: Divisions,
  province: string | null | undefined,
  city: string | null | undefined,
): string[] {
  return province && city ? (divisions[province]?.[city] ?? []) : [];
}

/**
 * 请求里带的产地字段: 逐级填, 上一级空着下一级就不带 (服务端同样拒绝跳级).
 * 没填的不带键, 不发 null.
 */
export function placeFields(place: Place): { province?: string; city?: string; county?: string } {
  const province = place.province || undefined;
  const city = province ? place.city || undefined : undefined;
  const county = city ? place.county || undefined : undefined;
  return {
    ...(province && { province }),
    ...(city && { city }),
    ...(county && { county }),
  };
}

/** 展示用: "山西省 晋中市 灵石县"; 没填省的旧数据退回自由文本 region; 都没有为空串. */
export function placeLabel(source: Place & { region?: string | null }): string {
  const { province, city, county } = placeFields(source);
  if (!province) return source.region ?? "";
  return [province, city, county].filter(Boolean).join(" ");
}
