ALTER TABLE wallet
    ADD COLUMN IF NOT EXISTS currency CHAR(3) NOT NULL DEFAULT 'USD';

ALTER TABLE purchase
    ADD COLUMN IF NOT EXISTS currency CHAR(3) NOT NULL DEFAULT 'USD',
    ADD COLUMN IF NOT EXISTS exchange_rate NUMERIC,
    ADD COLUMN IF NOT EXISTS rate_source VARCHAR,
    ADD COLUMN IF NOT EXISTS rate_at TIMESTAMPTZ;

-- Unlogged cache: not crash-safe and not replicated, but rates are also
-- denormalized onto purchase rows, so a lost cache row is harmless.
CREATE UNLOGGED TABLE IF NOT EXISTS exchange_rate_cache (
    from_currency CHAR(3) NOT NULL,
    to_currency   CHAR(3) NOT NULL,
    rate_hour     TIMESTAMPTZ NOT NULL,
    rate          NUMERIC NOT NULL,
    source        VARCHAR NOT NULL,
    fetched_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    stale         BOOL NOT NULL DEFAULT FALSE,
    PRIMARY KEY (from_currency, to_currency, rate_hour)
);

CREATE OR REPLACE VIEW purchase_items AS
    SELECT p.id as purchase_id,
           p.store as store,
           c.name as category,
           p.purchase_date as purchase_date,
           p.user_id as user_id,
           p.wallet_id as wallet_id,
           p.description as description,
           p.notes as notes,
           p.created_at as created_at,
           p.updated_at as updated_at,
           p.currency as source_currency,
           w.currency as wallet_currency,
           p.exchange_rate as exchange_rate,
           p.rate_source as rate_source,
           p.rate_at as rate_at,
           sum(i.quantity * i.unit_price) as total,
           sum(i.quantity * i.unit_price) * COALESCE(p.exchange_rate, 1) as total_wallet,
           json_agg(
                json_build_object(
                    'sequence', i.item_sequence,
                    'name', i.name,
                    'unit_price', i.unit_price,
                    'quantity', i.quantity,
                    'item_type', i.item_type
                )
           ) as items
    FROM purchase p
        JOIN item i on p.id = i.purchase_id
        JOIN category c on p.category_id = c.id
        JOIN wallet w on p.wallet_id = w.id
    WHERE p.deleted_at IS NULL
    GROUP BY p.id, c.name, w.currency;
