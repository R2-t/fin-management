-- The role the training job runs as (D84 review finding).
--
-- Training is cross-user by design, so it cannot use the RLS-scoped `analytics`
-- role: with no tenant stated it sees nothing. The first version of the job
-- simply used the owner connection, which handed the sidecar full DML on every
-- tenant -- including `users.password_hash`, which U26 deliberately revokes from
-- `analytics`. D4's premise is that a compromised sidecar cannot corrupt a
-- purchase; an owner credential deletes every purchase.
--
-- This role is the narrowest thing that can do the job:
--
--   BYPASSRLS      reads across tenants, which is the point of the model
--   SELECT         only the three tables labels are built from
--   INSERT,UPDATE  only the model registry it writes
--   nothing else   no DELETE anywhere, no access to money or identity tables
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trainer') THEN
        -- Development value, in version control the way CI keeps its test
        -- password. Production must override it:
        --     ALTER ROLE trainer PASSWORD '<secret>';
        CREATE ROLE trainer LOGIN BYPASSRLS PASSWORD 'trainer-dev-only';
    END IF;
END
$$;

GRANT USAGE ON SCHEMA public TO trainer;

-- The label source: what a purchase was bought for, and which category the user
-- finally left on it.
GRANT SELECT ON purchase, category, merchant TO trainer;

-- The registry it writes. No DELETE: a model is superseded, never removed, so a
-- correction or a rollback stays explainable.
GRANT SELECT, INSERT, UPDATE ON model_registry TO trainer;

-- Belt and braces: a future `GRANT ... ON ALL TABLES` must not quietly widen
-- this role past what the job needs.
REVOKE ALL ON users, user_profile, sessions, login_attempt FROM trainer;
REVOKE ALL ON wallet, ledger_entry, budget, income_schedule, transfer FROM trainer;
