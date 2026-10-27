-- 一铲多少吨: 现场铲车配煤的最小单位. 求解时每种煤要么不用、要么至少一铲, 且按整铲配.
-- NULL = 没设过, 读出时按默认值 (见 database.rs DEFAULT_SHOVEL_TONS).
ALTER TABLE web_user_state ADD COLUMN IF NOT EXISTS shovel_tons DOUBLE PRECISION;
