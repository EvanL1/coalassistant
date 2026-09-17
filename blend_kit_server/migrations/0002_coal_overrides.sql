-- 煤库数据覆盖层.
--
-- 基线是编译进二进制的 blend_kit_rs/data/coal_master.json (进 git, 可 diff 可 review)。
-- 本表只存"基线之外补充了什么", GET /api/master 返回 基线 + 本表 的合并结果。
-- 因此: 删掉一行 = 回滚一个值; 清空本表 = 回到 git 里那份基线。
--
-- 约束刻意写死在 schema 里, 与 scripts/check_master_data.mjs 的物理量程一致 ——
-- 2026-09-17 的调研表曾把年份 2026 填进岩相列, 应用层与数据库两道都要拦得住。
CREATE TABLE IF NOT EXISTS coal_overrides (
    coal_name  TEXT NOT NULL,
    field      TEXT NOT NULL CHECK (field IN ('S', 'A', 'V', 'G', 'Y', 'petro', 'CSR', 'M')),
    value      DOUBLE PRECISION NOT NULL CHECK (value >= 0),
    -- 来源/原始口径, 例如 "2026-09-17 Mysteel 规格 Y≥16"。保留原始 ≥/≤ 符号,
    -- 这样"界限值"与"实测值"之后仍分得清。
    source     TEXT NOT NULL,
    confidence TEXT NOT NULL DEFAULT 'medium'
        CHECK (confidence IN ('high', 'medium', 'low')),
    updated_by TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (coal_name, field),

    -- 各指标物理量程 (超出即录入错误, 与具体煤源无关)
    CONSTRAINT coal_overrides_value_in_range CHECK (
        CASE field
            WHEN 'S'     THEN value <= 10
            WHEN 'A'     THEN value <= 50
            WHEN 'V'     THEN value <= 50
            WHEN 'G'     THEN value <= 100
            WHEN 'Y'     THEN value <= 50
            WHEN 'petro' THEN value <= 1
            WHEN 'CSR'   THEN value <= 100
            WHEN 'M'     THEN value <= 30
            -- 必须有 ELSE: CASE 无分支命中时返回 NULL, 而 CHECK 对 NULL 判定为
            -- **通过**。将来往上面的 field IN (...) 加指标却忘了加这里的分支,
            -- 量程校验会静默失效 —— 正是这张表要拦的那类脏数据。
            ELSE false
        END
    )
);
