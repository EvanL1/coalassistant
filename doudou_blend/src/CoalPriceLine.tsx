import type { CardPrice } from "./coalPrices";

function priceChange(change: number | null): string {
  if (change == null) return "";
  const rounded = Math.round(Math.abs(change));
  if (rounded === 0) return "持平";
  return `${change > 0 ? "▲" : "▼"}${rounded}`;
}

export function CoalPriceLine({ price, name, onOpen }: {
  price: CardPrice;
  /** 与 onOpen 一起给: 按钮的无障碍名 "查看{name}估价走势". */
  name?: string;
  /** 给了就变成按钮, 点开走势; 点击不冒泡到卡片 (卡片自己点开的是编辑器). */
  onOpen?: () => void;
}) {
  if (price.kind === "unavailable") {
    return <div style={{ fontSize: 12, color: "var(--c-text-3)", marginTop: 8 }}>估价暂不可用</div>;
  }
  const trend = price.kind === "snapshot" ? priceChange(price.change) : "";
  const text = (
    <>
      估价 {Math.round(price.price).toLocaleString("zh-CN")} 元/吨
      {trend && `  ${trend}`}
      {price.kind === "live" && price.modified && " · 按修改值估"}
      {price.missing.length > 0 && ` · 缺 ${price.missing.join("、")}`}
    </>
  );
  if (onOpen) {
    return (
      <button type="button" className="coal-price-line" aria-label={`查看${name}估价走势`}
        onClick={(event) => { event.stopPropagation(); onOpen(); }}>
        {text}
      </button>
    );
  }
  return <div style={{ fontSize: 12, color: "var(--c-text-2)", marginTop: 8 }}>{text}</div>;
}
