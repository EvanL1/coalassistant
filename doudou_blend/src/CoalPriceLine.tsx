import type { CardPrice } from "./coalPrices";
import { changeMark, yuan } from "./priceForecast";

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
  const trend = price.kind === "snapshot" && price.change != null ? changeMark(price.change) : "";
  const forecast = price.kind === "snapshot" ? price.forecast : undefined;
  const text = (
    <>
      估价 {yuan(price.price)} 元/吨
      {trend && `  ${trend}`}
      {forecast != null && ` · 一周后约 ${yuan(forecast)}`}
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
