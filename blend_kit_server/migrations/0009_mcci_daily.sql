-- 钢铁网国产炼焦煤价格指数 (MCCI) 日度值: 服务端每天拉一次 (见 src/mcci.rs), 估价拿每条序列
-- 最新值当分煤种基准, 旧报价按同煤种序列推算到今天. 单位 元/湿吨 (现金); 标准品指标在
-- master price_factor.index_series. 整段历史留着, 将来回测估价与煤价误差要用.
CREATE TABLE IF NOT EXISTS mcci_daily (
    series_code TEXT NOT NULL,
    trade_date DATE NOT NULL,
    value DOUBLE PRECISION NOT NULL CHECK (value > 0),
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (series_code, trade_date)
);
