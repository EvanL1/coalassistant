-- 基准报价加 形态 / 地区, 与煤种并列: 估价按 煤种 → 形态 → 地区 逐层对标.
ALTER TABLE price_benchmarks ADD COLUMN IF NOT EXISTS form TEXT;
ALTER TABLE price_benchmarks ADD COLUMN IF NOT EXISTS region TEXT;
