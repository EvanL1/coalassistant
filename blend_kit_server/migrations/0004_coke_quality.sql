-- 历史方案补焦炭强度与炼焦条件, 全部选填. 只加可空列, 不动存量数据.
-- 炼焦条件 (装煤密度/结焦时间/炉温) 约占焦炭强度差异的三成, 以后拟合 CSR 时
-- 要靠它区分"煤的原因"和"炉子的原因".
ALTER TABLE web_blend_history
    ADD COLUMN IF NOT EXISTS cri_measured DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS m40_measured DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS m10_measured DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS bulk_density DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS coking_hours DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS flue_temp DOUBLE PRECISION;
