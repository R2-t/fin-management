-- Phase 3: the sidecar's database access, and the tables it owns.
--
-- The role is read-only on the application's data and can write only its own
-- tables (D4). It exists so a compromised sidecar cannot corrupt a purchase,
-- a ledger entry or a balance: it can read what it needs to classify, and
-- write only what it produces.

-- Idempotent because `DROP SCHEMA public CASCADE` does not drop roles, so a
-- database replayed from empty would otherwise fail here on a role that
-- outlived its schema.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'analytics') THEN
        -- The password is a development value, kept in version control the way
        -- CI keeps its test database password. Production must override it:
        --     ALTER ROLE analytics PASSWORD '<secret>';
        CREATE ROLE analytics LOGIN PASSWORD 'analytics-dev-only';
    END IF;
END
$$;

GRANT USAGE ON SCHEMA public TO analytics;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO analytics;

-- No DELETE anywhere: a model or a label is superseded, never removed, so a
-- result can be explained later.
-- No write access to purchase, item, ledger_entry, wallet or budget: those
-- belong to the application.

-- ---------------------------------------------------------------------------
-- Classification
-- ---------------------------------------------------------------------------

-- What the system decided about a purchase's category, and where the decision
-- came from. `source` distinguishes a model's answer from a rule's from the
-- user's own choice, so a later correction can be told apart from a first
-- guess.
CREATE TABLE IF NOT EXISTS purchase_classification (
    purchase_id            UUID PRIMARY KEY REFERENCES purchase(id) ON DELETE CASCADE,
    predicted_category_id  INTEGER REFERENCES category(id),
    -- Dimensionless, like every other probability in this system. Never money.
    confidence             REAL,
    model_version          TEXT,
    source                 TEXT NOT NULL CHECK (source IN ('model', 'rule', 'user', 'pending')),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- The highest-value label source: every time the user corrects a prediction,
-- the pair (predicted, corrected) is what the model learns from.
CREATE TABLE IF NOT EXISTS classification_feedback (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    purchase_id            UUID NOT NULL REFERENCES purchase(id) ON DELETE CASCADE,
    predicted_category_id  INTEGER REFERENCES category(id),
    corrected_category_id  INTEGER NOT NULL REFERENCES category(id),
    model_version          TEXT,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Training pulls labels in time order, never at random: a random split would
-- leak the future into the past.
CREATE INDEX IF NOT EXISTS classification_feedback_time_idx
    ON classification_feedback (created_at);

-- ---------------------------------------------------------------------------
-- Model registry
-- ---------------------------------------------------------------------------

-- Every trained model, with the metrics it was evaluated on. A candidate is
-- registered inactive and promoted only if it beats the active one on every
-- reported metric (D48); rollback is flipping the flag back.
CREATE TABLE IF NOT EXISTS model_registry (
    model_version  TEXT PRIMARY KEY,
    trained_at     TIMESTAMPTZ NOT NULL,
    metrics        JSONB NOT NULL,
    active         BOOLEAN NOT NULL DEFAULT FALSE
);

-- At most one model is active, enforced by the database rather than by the
-- training job getting it right.
CREATE UNIQUE INDEX IF NOT EXISTS model_registry_active_idx
    ON model_registry (active)
    WHERE active;

-- The sidecar's write access, granted only after its tables exist. SELECT is
-- included because the earlier `ON ALL TABLES` grant ran before these tables
-- existed.
GRANT SELECT, INSERT, UPDATE ON purchase_classification, classification_feedback, model_registry TO analytics;
