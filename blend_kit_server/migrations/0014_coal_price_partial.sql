-- 输入不全 (CCP 取不到、某条指数序列还没历史) 时的临时快照 partial: 不覆盖当天已有的完整快照,
-- 自己可被之后的实时快照或补算覆盖, 不会过了零点就定格. 只放宽取值, 上一版代码照常可用.
DO $$
DECLARE
    constraint_name TEXT;
BEGIN
    FOR constraint_name IN
        SELECT conname FROM pg_constraint
        WHERE conrelid = 'coal_price_daily'::regclass
          AND contype = 'c'
          AND pg_get_constraintdef(oid) LIKE '%source%'
    LOOP
        EXECUTE format('ALTER TABLE coal_price_daily DROP CONSTRAINT %I', constraint_name);
    END LOOP;
END $$;
ALTER TABLE coal_price_daily
    ADD CONSTRAINT coal_price_daily_source_check
    CHECK (source IN ('live', 'backfill', 'partial'));
