-- 管理端全局设置 (键值). 目前只有 rank_interaction_k: 求解时对未自带煤阶交互的请求注入.
CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
