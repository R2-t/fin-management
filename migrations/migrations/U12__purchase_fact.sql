-- Phase 0.10: append-only analytics projection of purchases.
--
-- The OLTP `purchase` table is mutable and soft-deleted, so it is NOT made a
-- hypertable: every unique index on a hypertable must include the partition
-- key, which would break `item.purchase_id -> purchase(id)`. Instead this
-- derived table has no foreign keys and is fully rebuildable.
--
-- Money is materialised here (unlike OLTP, where totals stay derived): the
-- projection is explicitly a reporting artifact, per docs/MONEY.md.
CREATE TABLE IF NOT EXISTS purchase_fact (
    occurred_at          TIMESTAMPTZ NOT NULL,
    purchase_id          UUID NOT NULL,
    version              INT NOT NULL,
    user_id              UUID NOT NULL,
    wallet_id            UUID NOT NULL,
    category_id          INT,
    merchant_id          UUID,
    source               TEXT NOT NULL DEFAULT 'purchase',
    currency             CHAR(3) NOT NULL,
    amount_minor         BIGINT NOT NULL,
    exchange_rate_scaled BIGINT,
    base_currency        CHAR(3) NOT NULL,
    amount_base_minor    BIGINT NOT NULL,
    -- The partition key must be part of the primary key.
    PRIMARY KEY (occurred_at, purchase_id, version)
);

SELECT create_hypertable(
    'purchase_fact',
    by_range('occurred_at', INTERVAL '1 month'),
    if_not_exists => TRUE
);

CREATE INDEX IF NOT EXISTS purchase_fact_user_time_idx
    ON purchase_fact (user_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS purchase_fact_user_category_idx
    ON purchase_fact (user_id, category_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS purchase_fact_user_merchant_idx
    ON purchase_fact (user_id, merchant_id, occurred_at DESC);
