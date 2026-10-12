-- 密钥用途: 一把钥匙只开一扇门 (最小权限).
--   data = 写煤库覆盖层 (调研数据录入, /api/master/coals)
--   mcp  = 只读 + 求解 (手机 Agent 经 /mcp 调配煤), 不能写煤库
-- 存量密钥都是数据录入用的, 默认 data, 行为不变.
ALTER TABLE data_api_keys
    ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'data'
        CHECK (scope IN ('data', 'mcp'));
