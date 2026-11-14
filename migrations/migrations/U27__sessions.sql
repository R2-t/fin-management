-- Sessions: a logged-in browser's proof of identity.
--
-- The cookie carries 32 random bytes; this table stores only
-- `sha256(token)`, so a reader of the table cannot mint a session. Lookup is
-- one index hit on `id_hash`, and no secret byte is ever compared in
-- application code -- Postgres compares the hash.
--
-- Absolute expiry, no sliding window: a sliding window writes a row on every
-- request and makes "when does this session end" unanswerable. There is no
-- `last_seen_at` for the same reason -- it would be written and never read.
CREATE TABLE IF NOT EXISTS sessions (
    id_hash    BYTEA PRIMARY KEY,
    user_id    UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions (expires_at);

REVOKE ALL ON sessions FROM analytics;
