-- 估价快照的来历: live = 当天每小时重算、过零点定格的实时快照; backfill = 事后按"当天能拿到的
-- 指数与报价"补算的 (服务停机的日子、新加的煤、上线前的一年). 两种都存下就不改.
ALTER TABLE coal_price_daily
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'live'
    CHECK (source IN ('live', 'backfill'));
