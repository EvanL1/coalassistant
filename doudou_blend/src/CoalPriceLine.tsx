import type { CardPrice } from "./coalPrices";

function priceChange(change: number | null): string {
  if (change == null) return "";
  const rounded = Math.round(Math.abs(change));
  if (rounded === 0) return "持平";
  return `${change > 0 ? "▲" : "▼"}${rounded}`;
}

export function CoalPriceLine({ price }: { price: CardPrice }) {
  if (price.kind === "unavailable") {
    return <div style={{ fontSize: 12, color: "var(--c-text-3)", marginTop: 8 }}>估价暂不可用</div>;
  }
  const trend = price.kind === "snapshot" ? priceChange(price.change) : "";
  return (
    <div style={{ fontSize: 12, color: "var(--c-text-2)", marginTop: 8 }}>
      估价 {Math.round(price.price).toLocaleString("zh-CN")} 元/吨
      {trend && `  ${trend}`}
      {price.kind === "live" && price.modified && " · 按修改值估"}
      {price.missing.length > 0 && ` · 缺 ${price.missing.join("、")}`}
    </div>
  );
}
