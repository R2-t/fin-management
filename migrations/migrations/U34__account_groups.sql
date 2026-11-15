-- Account groups: a named set of wallets, for reporting.
--
-- The app never sums across currencies (ADR-0002), so a group is not a
-- net-worth figure: it is a filter. Its purpose is to answer "how are my
-- savings doing" without reading three wallets and adding them by hand.
CREATE TABLE IF NOT EXISTS account_group (
    id         UUID PRIMARY KEY,
    user_id    UUID NOT NULL,
    name       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);

-- One live name per user. Case-insensitive because two groups differing only in
-- case are a mistake, not a distinction.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_account_group_name
    ON account_group (user_id, lower(btrim(name)))
    WHERE deleted_at IS NULL;

-- A wallet belongs to at most one group. NULL means ungrouped, which is the
-- default: grouping is opt-in and never assumed.
ALTER TABLE wallet
    ADD COLUMN IF NOT EXISTS account_group_id UUID REFERENCES account_group(id);

CREATE INDEX IF NOT EXISTS idx_wallet_account_group ON wallet (account_group_id);

-- The sidecar does not need groups; keep its access as narrow as it is.
REVOKE ALL ON account_group FROM analytics;
