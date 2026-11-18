-- A finding about records that look wrong: a duplicated Purchase, a charge far
-- above the usual amount, or a subscription nobody remembers signing up for.
--
-- The detector that produces a finding owns its evidence. `key` is that
-- evidence in one comparable string (amount, day, cadence, price level), so
-- detecting the same unchanged record again is the same finding. It is not the
-- record's version: a light edit bumps `purchase.version` but must not re-raise
-- a finding the user already dismissed.
CREATE TABLE IF NOT EXISTS anomaly (
    id               UUID PRIMARY KEY,
    user_id          UUID NOT NULL,
    kind             TEXT NOT NULL, -- 'duplicate' | 'amount_outlier' | 'unintended_recurring'
    subject_type     TEXT NOT NULL, -- 'purchase' | 'merchant' | 'cadence'
    subject_id       UUID,          -- null for a merchant-level finding
    title            TEXT NOT NULL,
    detail           JSONB NOT NULL, -- the evidence: amounts, dates, the numbers
    severity         TEXT NOT NULL DEFAULT 'notice'
                     CHECK (severity IN ('notice', 'warning')),
    detected_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    key              TEXT NOT NULL,
    dismissed_at     TIMESTAMPTZ,
    dismissed_reason TEXT
);

-- Two partial indexes because Postgres treats NULLs as distinct: a plain
-- UNIQUE would let a merchant-level finding insert a new row on every sweep.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_anomaly_with_subject
    ON anomaly (user_id, kind, subject_type, subject_id, key)
    WHERE subject_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uniq_anomaly_without_subject
    ON anomaly (user_id, kind, subject_type, key)
    WHERE subject_id IS NULL;

-- `GET /anomaly` defaults to open findings, newest first.
CREATE INDEX IF NOT EXISTS idx_anomaly_open
    ON anomaly (user_id, detected_at DESC, id DESC)
    WHERE dismissed_at IS NULL;

-- `state=dismissed` lists the other half in the same order.
CREATE INDEX IF NOT EXISTS idx_anomaly_dismissed
    ON anomaly (user_id, detected_at DESC, id DESC)
    WHERE dismissed_at IS NOT NULL;

-- Detectors run in the application (D4/D66), so the sidecar neither writes nor
-- reads findings. Keep its access as narrow as it is.
REVOKE ALL ON anomaly FROM analytics;
