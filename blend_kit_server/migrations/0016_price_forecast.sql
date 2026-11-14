-- 一周后的价格预测 (见 src/forecast.rs): 按钢铁网指数各序列的历史, 预测 5 个交易日后的指数值,
-- 再按预测的指数重估共享煤库. 只作参考展示, 不进求解.

-- 指数各序列的预测, 每个 "所依据的最新一期" 存一条, 留作日后对照实际值.
CREATE TABLE IF NOT EXISTS mcci_forecast (
    series_code TEXT NOT NULL,
    -- 预测所依据的最新一期指数的日期与当天的值.
    made_on DATE NOT NULL,
    base_value DOUBLE PRECISION NOT NULL CHECK (base_value > 0),
    -- 预测的是哪天、多少 (元/湿吨).
    target_date DATE NOT NULL,
    value DOUBLE PRECISION NOT NULL CHECK (value > 0),
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (series_code, made_on)
);

-- 每种煤一周后的估价, 随当天的实时快照一起存; 补算的行没有 (NULL).
ALTER TABLE coal_price_daily
    ADD COLUMN IF NOT EXISTS forecast_price DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS forecast_day DATE;
