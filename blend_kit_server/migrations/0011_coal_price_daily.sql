-- 每小时重算当天 (北京时间) 的共享煤库估价, 过零点定格; 不含用户化验值覆盖.
-- 保留基准名和当时的生效价, 区分行情变动与对标变更; 历史不清理.
CREATE TABLE IF NOT EXISTS coal_price_daily (
    day DATE NOT NULL,
    name TEXT NOT NULL,
    price DOUBLE PRECISION NOT NULL,
    points DOUBLE PRECISION NOT NULL,
    benchmark TEXT,
    benchmark_price DOUBLE PRECISION,
    missing JSONB NOT NULL DEFAULT '[]',
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (day, name)
);
CREATE INDEX IF NOT EXISTS coal_price_daily_name_day ON coal_price_daily (name, day);
