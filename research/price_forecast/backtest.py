"""钢铁网炼焦煤指数 (MCCI) 时序预测的滚动回测.

问题: 只用指数自身的历史, 时序方法能不能比 "未来 = 今天" (随机游走) 预测得更准?

做法: 每个交易日 t 收盘后 (指数 17:30 发布) 只用 t 及以前的数据定参数、出预测, 对 t+h 的真值打分.
所有模型都在对数价上做, 误差换回 元/吨. 参数全部按 "当时能看到的数据" 估, 没有未来信息:
直推回归的训练样本只取目标值已在 t 前实现的 (起点 <= t-h).

用法: uv run --no-project --with numpy --with pandas python -I backtest.py <mcci_history.csv> [结果目录]
"""
import math
import sys
from pathlib import Path

import numpy as np
import pandas as pd

HORIZONS = (1, 5, 10, 20)
WARMUP = 500  # 前两年只用来定参数, 不打分
MAX_LAG = 20
ROLL = 250  # 滚动窗口版本: 只用最近一年
COMPOSITE = "LJM_ZH"
NAMES = {
    "LJM_ZH": "综合", "LJM_DLZJM": "低硫主焦", "LJM_ZLZJM": "中硫主焦", "LJM_GLZJM": "高硫主焦",
    "LJM_FM": "肥煤", "LJM_OTJM": "1/3焦", "LJM_SM": "瘦煤", "LJM_QM": "气煤",
}
RIDGE = 1e-10
SPLIT_YEAR = 2022  # 2023 起指数明显变平滑 (零变动天数骤降), 分两期看


def load(path: str) -> pd.DataFrame:
    """读成 日期 × 序列 的价格表 (只留 8 条都有值的交易日)."""
    raw = pd.read_csv(path, parse_dates=["date"])
    table = raw.pivot(index="date", columns="series", values="value").sort_index()
    return table[[code for code in NAMES if code in table.columns]].dropna()


def change(logp: np.ndarray, lag: int) -> np.ndarray:
    """过去 lag 天的对数涨幅; 前 lag 天没有, 记 0 (都落在预热期内)."""
    out = np.zeros_like(logp)
    out[lag:] = logp[lag:] - logp[:-lag]
    return out


def solve(xx: np.ndarray, xy: np.ndarray) -> np.ndarray:
    return np.linalg.solve(xx + RIDGE * np.eye(len(xy)), xy)


def direct_forecast(features: np.ndarray, logp: np.ndarray, horizon: int, window: int | None) -> np.ndarray:
    """直推回归: 未来 h 天涨幅 ~ 特征. 返回每个起点 t 的预测涨幅 (不可预测的起点为 nan).

    用累加的 X'X、X'y: 起点 t 的系数只用起点 <= t-h 的样本 (目标值在 t 当天已经实现)."""
    total = len(logp)
    target = np.zeros(total)
    target[: total - horizon] = logp[horizon:] - logp[:-horizon]
    usable = np.zeros(total, dtype=bool)
    usable[MAX_LAG : total - horizon] = True
    x = np.where(usable[:, None], features, 0.0)
    cum_xx = np.cumsum(x[:, :, None] * x[:, None, :], axis=0)
    cum_xy = np.cumsum(x * target[:, None], axis=0)
    pred = np.full(total, np.nan)
    for t in range(WARMUP, total):
        last = t - horizon
        xx, xy = cum_xx[last], cum_xy[last]
        if window is not None and last - window >= 0:
            xx, xy = xx - cum_xx[last - window], xy - cum_xy[last - window]
        pred[t] = features[t] @ solve(xx, xy)
    return pred


def arima_forecast(logp: np.ndarray, order: int) -> dict[int, np.ndarray]:
    """ARIMA(p,1,0): 日涨幅对自己前 p 天回归, 逐日递推到 h 天再累加."""
    total = len(logp)
    diff = change(logp, 1)
    lags = np.column_stack([np.ones(total)] + [np.roll(diff, k) for k in range(order)])
    target = np.zeros(total)
    target[:-1] = diff[1:]
    usable = np.zeros(total, dtype=bool)
    usable[MAX_LAG : total - 1] = True
    x = np.where(usable[:, None], lags, 0.0)
    cum_xx = np.cumsum(x[:, :, None] * x[:, None, :], axis=0)
    cum_xy = np.cumsum(x * target[:, None], axis=0)
    preds = {h: np.full(total, np.nan) for h in HORIZONS}
    for t in range(WARMUP, total):
        beta = solve(cum_xx[t - 1], cum_xy[t - 1])
        recent = list(diff[t - order + 1 : t + 1][::-1])  # 最近的在前
        total_change = 0.0
        for step in range(1, max(HORIZONS) + 1):
            nxt = beta[0] + float(np.dot(beta[1:], recent[:order]))
            recent.insert(0, nxt)
            total_change += nxt
            if step in preds:
                preds[step][t] = total_change
    return preds


def holt_forecast(logp: np.ndarray) -> dict[int, np.ndarray]:
    """阻尼趋势指数平滑 (damped Holt). 参数网格同时滤波, 每个起点选 "到当天为止一步误差最小" 的那组."""
    grid = np.array([(a, b, p) for a in (0.5, 0.7, 0.9, 1.0) for b in (0.05, 0.1, 0.2, 0.4, 0.7)
                     for p in (0.8, 0.9, 0.95, 0.98)])
    alpha, beta, phi = grid[:, 0], grid[:, 1], grid[:, 2]
    level = np.full(len(grid), logp[0])
    trend = np.zeros(len(grid))
    sse = np.zeros(len(grid))
    preds = {h: np.full(len(logp), np.nan) for h in HORIZONS}
    for t in range(1, len(logp)):
        expect = level + phi * trend
        sse += (logp[t] - expect) ** 2
        new_level = alpha * logp[t] + (1 - alpha) * expect
        trend = beta * (new_level - level) + (1 - beta) * phi * trend
        level = new_level
        if t >= WARMUP:
            best = int(np.argmin(sse))
            for h in HORIZONS:
                damp = sum(phi[best] ** j for j in range(1, h + 1))
                preds[h][t] = level[best] + damp * trend[best] - logp[t]
    return preds


def all_forecasts(logs: pd.DataFrame, code: str) -> dict[str, dict[int, np.ndarray]]:
    """一条序列的全部模型 → {模型名: {h: 每个起点的预测对数涨幅}}."""
    logp = logs[code].to_numpy()
    comp = logs[COMPOSITE].to_numpy()
    ones = np.ones(len(logp))
    own = np.column_stack([ones, change(logp, 1), change(logp, 5), change(logp, 20)])
    cross = np.column_stack([own, change(comp, 1), change(comp, 5)])
    models = {
        "随机游走": {h: np.where(np.arange(len(logp)) >= WARMUP, 0.0, np.nan) for h in HORIZONS},
        "ARIMA(1,1,0)": arima_forecast(logp, 1),
        "ARIMA(5,1,0)": arima_forecast(logp, 5),
        "阻尼Holt": holt_forecast(logp),
        "直推回归": {h: direct_forecast(own, logp, h, None) for h in HORIZONS},
        "直推回归·近一年": {h: direct_forecast(own, logp, h, ROLL) for h in HORIZONS},
    }
    if code != COMPOSITE:
        models["直推+综合指数"] = {h: direct_forecast(cross, logp, h, None) for h in HORIZONS}
    return models


def dm_pvalue(loss_gap: np.ndarray, horizon: int) -> float:
    """Diebold-Mariano: 绝对误差之差的均值是否显著不为 0 (Newey-West, 重叠 h 天要去自相关)."""
    n = len(loss_gap)
    centered = loss_gap - loss_gap.mean()
    lags = 2 * (horizon - 1)
    var = centered @ centered / n
    for k in range(1, lags + 1):
        var += 2 * (1 - k / (lags + 1)) * (centered[k:] @ centered[:-k]) / n
    if var <= 0:
        return float("nan")
    stat = loss_gap.mean() / math.sqrt(var / n)
    return math.erfc(abs(stat) / math.sqrt(2))


def score(price: np.ndarray, dates: pd.DatetimeIndex, pred: np.ndarray, horizon: int) -> dict:
    """一个 (序列, 模型, h) 的成绩. 同时返回逐起点误差, 供分期与区间用.

    方向一律保守计: 到期价格没动算 "没说对"."""
    origins = np.arange(WARMUP, len(price) - horizon)
    now, future = price[origins], price[origins + horizon]
    forecast = now * np.exp(pred[origins])
    err, naive_err = forecast - future, now - future
    actual = np.sign(future - now)
    said = np.sign(forecast - now)
    momentum = np.sign(now - price[origins - horizon])  # 方向基线: 沿用过去 h 天的方向
    call = np.abs(pred[origins]) >= 0.01  # 模型喊 "涨跌超过 1%" 的时候
    calls = max(int(call.sum()), 1)
    return {
        "n": len(origins),
        "naive_mae": np.abs(naive_err).mean(),
        "mae": np.abs(err).mean(),
        "mape": (np.abs(err) / future).mean() * 100,
        "ratio": np.abs(err).mean() / np.abs(naive_err).mean(),
        "rmse_ratio": math.sqrt((err**2).mean() / (naive_err**2).mean()),
        "dm_p": dm_pvalue(np.abs(naive_err) - np.abs(err), horizon),
        "hit": ((said == actual) & (actual != 0)).mean() * 100,
        "momentum_hit": ((momentum == actual) & (actual != 0)).mean() * 100,
        "call_share": call.mean() * 100,
        "call_right": ((said == actual) & (actual != 0) & call).sum() / calls * 100,
        "call_wrong": ((said == -actual) & (actual != 0) & call).sum() / calls * 100,
        "_err": err, "_naive": naive_err, "_year": dates[origins].year.to_numpy(),
        "_logerr": np.log(future / forecast),
    }


def period_ratio(result: dict, mask: np.ndarray) -> float:
    return float(np.abs(result["_err"][mask]).mean() / np.abs(result["_naive"][mask]).mean())


def interval_check(result: dict, horizon: int) -> tuple[float, float]:
    """80% 区间: 起点 t 用 "t 前已实现的预测误差" 的 10%/90% 分位. 返回 (实际覆盖率%, 平均半宽%)."""
    logerr = result["_logerr"]
    inside, width = [], []
    for i in range(250 + horizon, len(logerr)):
        low, high = np.quantile(logerr[: i - horizon + 1], [0.1, 0.9])
        inside.append(low <= logerr[i] <= high)
        width.append((high - low) / 2)
    return float(np.mean(inside) * 100), float(np.mean(width) * 100)


def print_properties(logs: pd.DataFrame) -> None:
    diff = logs.diff().dropna()
    print("== 序列自身的性质 (日对数涨幅): 零变动天数% / 滞后 1 天自相关 ==")
    print(f"{'序列':<8}{'2018-22':>14}{'2023 起':>14}{'日波动%(全期)':>14}")
    early = diff[diff.index.year <= SPLIT_YEAR]
    late = diff[diff.index.year > SPLIT_YEAR]
    for code in logs.columns:
        cells = [f"{(part[code] == 0).mean() * 100:.0f} / {part[code].autocorr(1):.2f}" for part in (early, late)]
        print(f"{NAMES[code]:<8}{cells[0]:>14}{cells[1]:>14}{diff[code].std() * 100:>14.2f}")


def main() -> None:
    prices = load(sys.argv[1])
    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else None
    logs = np.log(prices)
    dates = prices.index
    print(f"数据: {dates[0].date()} ~ {dates[-1].date()}, {len(prices)} 个交易日; "
          f"打分区间自 {dates[WARMUP].date()} 起\n")
    print_properties(logs)

    rows, kept = [], {}
    for code in prices.columns:
        price = prices[code].to_numpy()
        for model, by_h in all_forecasts(logs, code).items():
            for h in HORIZONS:
                result = score(price, dates, by_h[h], h)
                kept[(code, model, h)] = result
                rows.append({"series": NAMES[code], "model": model, "h": h,
                             **{k: v for k, v in result.items() if not k.startswith("_")}})
    table = pd.DataFrame(rows)

    print("\n== 误差比 = 模型 MAE / 随机游走 MAE (<1 才算赢), 各序列的中位数 ==")
    pivot = table.pivot_table(index="model", columns="h", values="ratio", aggfunc="median")
    print(pivot.loc[[m for m in pivot.index if m != "随机游走"]].round(3).to_string())

    def best(code: str, h: int) -> dict:
        """最好的那个模型: 分指数用 "直推+综合指数", 综合指数自己用 "直推回归"."""
        return kept[(code, "直推回归" if code == COMPOSITE else "直推+综合指数", h)]

    print("\n== 逐序列, 最好的模型 (分指数: 直推+综合指数; 综合: 直推回归) ==")
    print(f"{'序列':<8}{'h':>3}{'MAE 元: 基线→模型':>20}{'误差比':>7}{'DM p':>7}"
          f"{'2020-22':>9}{'2023 起':>8}  | 方向说对%: 沿用动量 / 模型 | 喊单占比% 说对% 说反%")
    for code in prices.columns:
        for h in HORIZONS:
            r = best(code, h)
            early = r["_year"] <= SPLIT_YEAR
            print(f"{NAMES[code]:<8}{h:>3}{r['naive_mae']:>12.1f} → {r['mae']:<7.1f}{r['ratio']:>6.2f}"
                  f"{r['dm_p']:>8.3f}{period_ratio(r, early):>8.2f}{period_ratio(r, ~early):>8.2f}  |"
                  f"{r['momentum_hit']:>12.0f} / {r['hit']:<5.0f}|{r['call_share']:>8.0f}"
                  f"{r['call_right']:>7.0f}{r['call_wrong']:>6.0f}")

    print("\n== 分年误差比: 最好的模型, 各序列合计 (每年的 MAE 之和相比) ==")
    print(f"{'年份':<6}" + "".join(f"{f'h={h}':>8}" for h in HORIZONS))
    for year in sorted(set(best(COMPOSITE, 1)["_year"])):
        cells = []
        for h in HORIZONS:
            picked = [(best(c, h), best(c, h)["_year"] == year) for c in prices.columns]
            model_sum = sum(np.abs(r["_err"][m]).sum() for r, m in picked)
            naive_sum = sum(np.abs(r["_naive"][m]).sum() for r, m in picked)
            cells.append(f"{model_sum / naive_sum:>8.2f}")
        print(f"{year:<6}" + "".join(cells))

    print("\n== 80% 区间 (用过往误差分位): 覆盖率% / 半宽% —— 随机游走 vs 最好的模型 ==")
    print(f"{'序列':<8}" + "".join(f"{f'h={h}':>34}" for h in (5, 10, 20)))
    for code in prices.columns:
        cells = []
        for h in (5, 10, 20):
            nc, nw = interval_check(kept[(code, "随机游走", h)], h)
            mc, mw = interval_check(best(code, h), h)
            cells.append(f"{nc:.0f}/±{nw:.1f} vs {mc:.0f}/±{mw:.1f}")
        print(f"{NAMES[code]:<8}" + "".join(f"{c:>34}" for c in cells))

    if out_dir is not None:
        out_dir.mkdir(parents=True, exist_ok=True)
        table.round(4).to_csv(out_dir / "metrics.csv", index=False)
        print(f"\n明细已存 {out_dir / 'metrics.csv'}")


if __name__ == "__main__":
    main()
