-- Phase 0 hardening: durable rate history and honest resolution states.
--
-- H1. `exchange_rate_cache` is UNLOGGED (wiped on crash or immediate restart) and
-- was the only place a fetched rate ever lived, so a Purchase recorded with an
-- unresolved conversion could never be completed from data we already have.
-- `fx_rate_history` is a logged table written on every successful fetch.
--
-- H3. The projection now distinguishes three resolution states instead of two:
--   not_required  same currency as the wallet; no conversion was ever attempted
--   completed     a conversion was applied
--   unresolved    a conversion was attempted and failed; complete it later
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
                   * power(
                       10::numeric,
                       (COALESCE(cw.minor_unit, 2) - COALESCE(cs.minor_unit, 2))::numeric
                     )
               )::bigint
           END as total_wallet,
           CASE
               WHEN p.currency = w.currency THEN 'not_required'
               WHEN p.exchange_rate_scaled IS NOT NULL THEN 'completed'
               ELSE 'unresolved'
           END as resolution,
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
        LEFT JOIN currency cs on cs.code = p.currency
        LEFT JOIN currency cw on cw.code = w.currency
    WHERE p.deleted_at IS NULL
    GROUP BY p.id, c.name, w.currency, cs.minor_unit, cw.minor_unit;

CREATE TABLE IF NOT EXISTS fx_rate_history (
    from_currency CHAR(3) NOT NULL,
    to_currency   CHAR(3) NOT NULL,
    rate_hour     TIMESTAMPTZ NOT NULL,
    rate_scaled   BIGINT NOT NULL,
    source        VARCHAR NOT NULL,
    fetched_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (from_currency, to_currency, rate_hour)
);

COMMENT ON TABLE fx_rate_history IS
    'Durable rate history (logged). Written on every successful fetch; the unlogged '
    'exchange_rate_cache remains the hot lookup. Completion of unresolved purchases '
    'reads from here or from the provider.';