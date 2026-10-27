-- 基准报价: 分煤种的市场价 (出厂含税, 带报价日). 估价模块按"同煤种、指标最接近"的基准换算.
-- master.price_factor.benchmarks 是随版本发布的默认值; 这张表是管理员随时补录的新报价,
-- 不用重新部署. 同名取报价日最新的一条; 旧报价不删, 将来回测估价准不准要用.
CREATE TABLE IF NOT EXISTS price_benchmarks (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    coal_type TEXT,
    props JSONB NOT NULL,
    price DOUBLE PRECISION NOT NULL CHECK (price > 0),
    quoted_on DATE NOT NULL,
    source TEXT,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS price_benchmarks_name_quoted_on
    ON price_benchmarks (name, quoted_on DESC, created_at DESC);
