CREATE TABLE IF NOT EXISTS web_user_state (
    username TEXT PRIMARY KEY,
    coal_prefs JSONB NOT NULL DEFAULT '{}'::jsonb,
    contract JSONB,
    quantity DOUBLE PRECISION NOT NULL DEFAULT 3700 CHECK (quantity > 0),
    user_coals JSONB NOT NULL DEFAULT '[]'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS web_blend_history (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    contract_name TEXT NOT NULL,
    cost_cif DOUBLE PRECISION NOT NULL,
    total_quantity DOUBLE PRECISION,
    recipe_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    result_json JSONB,
    csr_measured DOUBLE PRECISION,
    s_measured DOUBLE PRECISION,
    a_measured DOUBLE PRECISION,
    v_measured DOUBLE PRECISION,
    g_measured DOUBLE PRECISION,
    y_measured DOUBLE PRECISION,
    m_measured DOUBLE PRECISION,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS web_blend_history_user_time_idx
    ON web_blend_history (username, occurred_at DESC);
