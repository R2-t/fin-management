-- Users: the row behind the user_id every other table already carries.
--
-- `id` IS the opaque user_id (D70). No new identity column, no remapping: the
-- UUID that tenancy already keys on becomes the primary key of this table.
--
-- `password_hash` is an Argon2id PHC string, which embeds its own parameters
-- so raising the cost later still verifies hashes written today. A row whose
-- hash is not a parseable PHC string cannot log in -- that is how a seeded or
-- disabled user is represented, rather than a flag nothing reads.
CREATE EXTENSION IF NOT EXISTS citext;

CREATE TABLE IF NOT EXISTS users (
    id            UUID PRIMARY KEY,
    email         CITEXT UNIQUE NOT NULL,
    password_hash TEXT   NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- No FK from user_profile (D75): the integration suite fabricates a fresh
-- random user_id per test at 97 call sites, and U7 seeded profile rows from
-- wallet/purchase user ids that have no users row, so an FK would fail on any
-- non-empty database.
--
-- No grant to the analytics role, and no `GRANT ... ON ALL TABLES`: U18's
-- blanket grant was a snapshot of the tables that existed then, so this table
-- stays invisible to the sidecar.
REVOKE ALL ON users FROM analytics;
