-- Phase 0 hardening: a Purchase whose conversion is unknown must never be recorded at parity.
--
-- The previous view coalesced a missing rate to 1:1, so a USD purchase landing in a EUR
-- wallet was recorded as if 1 USD = 1 EUR. That was unreachable only because the write
-- failed first; now that writes succeed without a rate, the view must be honest.
--
-- The view distinguishes three cases explicitly:
--   same currency            -> the amount passes through unchanged
--   different currency, rate -> convert with the purchase's own stored rate
--   different currency, none -> NO converted amount at all (unresolved)
DROP VIEW IF EXISTS purchase_items;
CREATE VIEW purchase_items AS
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
           p.exchange_rate_scaled as exchange_rate_scaled,
           p.rate_source as rate_source,
           p.rate_at as rate_at,
           p.merchant_id as merchant_id,
           p.classification_status as classification_status,
           sum(i.quantity * i.unit_price) as total,
           CASE
               WHEN p.currency = w.currency THEN sum(i.quantity * i.unit_price)::bigint
               WHEN p.exchange_rate_scaled IS NULL THEN NULL
               ELSE round(
                   sum(i.quantity * i.unit_price)::numeric
                   * p.exchange_rate_scaled
                   / 1000000000
               )::bigint
           END as total_wallet,
           (p.currency = w.currency OR p.exchange_rate_scaled IS NOT NULL) as conversion_resolved,
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
        LEFT JOIN category c on p.category_id = c.id
        JOIN wallet w on p.wallet_id = w.id
    WHERE p.deleted_at IS NULL
    GROUP BY p.id, c.name, w.currency;

-- The projection may now legitimately carry no converted amount, and records its own
-- resolution status so unresolved rows are queryable and excludable from aggregates.
ALTER TABLE purchase_fact ALTER COLUMN amount_base_minor DROP NOT NULL;
ALTER TABLE purchase_fact
    ADD COLUMN IF NOT EXISTS resolution TEXT NOT NULL DEFAULT 'completed';

-- Supports finding work for later rate completion without scanning the whole hypertable.
CREATE INDEX IF NOT EXISTS purchase_fact_unresolved_idx
    ON purchase_fact (user_id, occurred_at)
    WHERE resolution = 'unresolved';
