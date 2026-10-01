-- Per-user reporting preferences (Phase 0.3).
--
-- `timezone` drives local-calendar bucketing (time_bucket) and pay periods;
-- `base_currency` is the default reporting currency, overridable per wallet.
CREATE TABLE IF NOT EXISTS user_profile (
    user_id       UUID NOT NULL PRIMARY KEY,
    timezone      TEXT NOT NULL DEFAULT 'UTC',
    base_currency CHAR(3) NOT NULL DEFAULT 'USD',
    locale        TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed profiles for users that already have data, so reads do not have to
-- synthesise a default. New users get a synthesised default until they PUT.
INSERT INTO user_profile (user_id)
    SELECT DISTINCT user_id FROM wallet WHERE deleted_at IS NULL
    ON CONFLICT (user_id) DO NOTHING;

INSERT INTO user_profile (user_id)
    SELECT DISTINCT user_id FROM purchase WHERE deleted_at IS NULL
    ON CONFLICT (user_id) DO NOTHING;
