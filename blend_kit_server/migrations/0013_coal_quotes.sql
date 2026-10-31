-- 询价存档: 用户当场询来的出厂价与运费, 附服务端按这批指标 + 当天基准重算的同口径估价.
-- 只用于对比与以后校准估价, 不进煤池、不跨次沿用. 同一批 (batch_id) 多次精算以最后一次为准.
CREATE TABLE IF NOT EXISTS coal_quotes (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    quoted_on DATE NOT NULL,
    name TEXT NOT NULL,
    coal_type TEXT,
    form TEXT,
    region TEXT,
    fob DOUBLE PRECISION NOT NULL CHECK (fob > 0),
    frt DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (frt >= 0),
    props JSONB NOT NULL,
    changed JSONB NOT NULL DEFAULT '[]',
    estimate DOUBLE PRECISION,
    points DOUBLE PRECISION,
    benchmark TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (username, batch_id, name)
);
CREATE INDEX IF NOT EXISTS coal_quotes_day ON coal_quotes (quoted_on);
CREATE INDEX IF NOT EXISTS coal_quotes_user_name ON coal_quotes (username, name, created_at);
