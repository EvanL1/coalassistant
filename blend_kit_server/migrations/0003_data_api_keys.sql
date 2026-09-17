-- 数据更新接口的 API 密钥, 在管理端页面生成/命名/销毁.
--
-- **只存哈希, 明文永不落库。** 明文在创建时返回一次, 之后任何接口都取不回来 ——
-- 数据库泄露时这些钥匙仍然用不了, 与密码存储同理。
--
-- 销毁采用软删除 (revoked_at 置时间戳) 而不是 DELETE: 被吊销的密钥曾经写过的
-- coal_overrides.updated_by 仍要查得到是谁。
CREATE TABLE IF NOT EXISTS data_api_keys (
    -- 与 web_blend_history 一致用 TEXT 存 UUID, 避免为此开 sqlx 的 uuid feature
    id          TEXT PRIMARY KEY,
    -- 人给的名字, 用来认出"这把是谁的", 不是秘密
    name        TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 64),
    -- 明文的 SHA-256 十六进制. 鉴权时对来访密钥做同样的哈希再查这一列
    key_hash    TEXT NOT NULL UNIQUE,
    -- 明文前 8 位, 仅用于在管理端列表里认领 ("dk_3f9a…"), 不足以还原密钥
    key_prefix  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 最近一次成功调用, 用来判断"这把还在用吗 / 能不能销毁"
    last_used_at TIMESTAMPTZ,
    -- 非空即已销毁; 软删除保留历史归属
    revoked_at  TIMESTAMPTZ
);

-- 鉴权是每次写请求的必经之路, 且只认未销毁的密钥
CREATE INDEX IF NOT EXISTS data_api_keys_active_idx
    ON data_api_keys (key_hash) WHERE revoked_at IS NULL;
