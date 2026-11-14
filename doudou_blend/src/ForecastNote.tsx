/** 走势弹层里的一行预测: 一周后大约多少、比现在涨跌多少. */
import { changeMark, yuan, type PriceForecast } from "./priceForecast";

export function ForecastNote({ forecast }: { forecast: PriceForecast }) {
  return (
    <p className="price-forecast">
      预测一周后 ({forecast.date.slice(5)}) 约 <strong>{yuan(forecast.price)}</strong> 元/吨,
      较当前 {changeMark(forecast.price - forecast.base)}
      <span> · 按指数近期走势推算, 仅供参考</span>
    </p>
  );
}
