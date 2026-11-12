/**
 * 产地选择: 省 / 市 / 县三个下拉框逐级联动, 只能从行政区划名录里选, 不能手打.
 * 换了省, 市和县清空; 换了市, 县清空. 知道到哪一级就选到哪一级.
 */
import { useEffect, useState } from "react";
import type { CSSProperties } from "react";
import { citiesOf, countiesOf, loadDivisions, provincesOf } from "./domain/place";
import type { Divisions, Place } from "./domain/place";

interface Props {
  value: Place;
  onChange: (place: Place) => void;
  /** 三个下拉框各自的 className 与 style (录入界面的输入框样式不一样). */
  className?: string;
  style?: CSSProperties;
  /** 读屏用的前缀, 如 "产地" → "产地省份". */
  label?: string;
}

export function PlaceSelect({ value, onChange, className, style, label = "产地" }: Props) {
  const [divisions, setDivisions] = useState<Divisions | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    loadDivisions()
      .then((loaded) => alive && setDivisions(loaded))
      .catch(() => alive && setFailed(true));
    return () => {
      alive = false;
    };
  }, []);

  const province = value.province ?? "";
  const city = value.city ?? "";
  const county = value.county ?? "";
  const cities = divisions ? citiesOf(divisions, province) : [];
  const counties = divisions ? countiesOf(divisions, province, city) : [];
  const waiting = divisions == null;

  // 禁用的下拉框上的悬停提示在手机上看不到, 失败就直接写出来.
  if (failed) {
    return (
      <span role="alert" style={{ fontSize: 12, color: "var(--c-danger)" }}>
        行政区划名录加载失败, 请刷新页面重试
      </span>
    );
  }

  return (
    <>
      <select
        className={className}
        style={style}
        aria-label={`${label}省份`}
        value={province}
        disabled={waiting}
        onChange={(event) => onChange({ province: event.target.value || null, city: null, county: null })}
      >
        <option value="">{waiting ? "加载中…" : "省 (未选)"}</option>
        {divisions && provincesOf(divisions).map((name) => (
          <option key={name} value={name}>{name}</option>
        ))}
      </select>
      <select
        className={className}
        style={style}
        aria-label={`${label}城市`}
        value={city}
        disabled={cities.length === 0}
        onChange={(event) => onChange({ province, city: event.target.value || null, county: null })}
      >
        <option value="">市 (未选)</option>
        {cities.map((name) => (
          <option key={name} value={name}>{name}</option>
        ))}
      </select>
      <select
        className={className}
        style={style}
        aria-label={`${label}区县`}
        value={county}
        disabled={counties.length === 0}
        onChange={(event) => onChange({ province, city, county: event.target.value || null })}
      >
        <option value="">县 (未选)</option>
        {counties.map((name) => (
          <option key={name} value={name}>{name}</option>
        ))}
      </select>
    </>
  );
}
