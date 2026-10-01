-- Phase 0.8: analytics outbox and job queue.
--
-- The write path inserts a small outbox row in the same transaction as the
-- OLTP mutation. A Rust projector claims batches (FOR UPDATE SKIP LOCKED) and
-- rebuilds `purchase_fact`. Events carry ids only, so re-reading current state
-- makes processing idempotent and order-insensitive.
CREATE TABLE IF NOT EXISTS analytics_outbox (
    id                BIGSERIAL NOT NULL PRIMARY KEY,
    user_id           UUID NOT NULL,
    aggregate_type    TEXT NOT NULL,
    aggregate_id      UUID NOT NULL,
    aggregate_version INT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at      TIMESTAMPTZ,
    -- Set when a projector claims the row; stale claims are re-claimable.
    claimed_at        TIMESTAMPTZ,
    attempts          INT NOT NULL DEFAULT 0,
    last_error        TEXT
);

CREATE INDEX IF NOT EXISTS analytics_outbox_pending_idx
    ON analytics_outbox (id)
    WHERE processed_at IS NULL;

COMMENT ON COLUMN analytics_outbox.claimed_at IS
    'Projector claim timestamp; a claim older than the retry window is reclaimable.';

-- Work for the Python sidecar (classification, rescoring, training). Rust owns
-- the table; Python claims rows with SKIP LOCKED.
CREATE TABLE IF NOT EXISTS analytics_job (
    id          BIGSERIAL NOT NULL PRIMARY KEY,
    user_id     UUID NOT NULL,
    kind        TEXT NOT NULL,
    payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
    status      TEXT NOT NULL DEFAULT 'pending',
    attempts    INT NOT NULL DEFAULT 0,
    last_error  TEXT,
    run_after   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS analytics_job_pending_idx
    ON analytics_job (kind, run_after)
    WHERE status = 'pending';
