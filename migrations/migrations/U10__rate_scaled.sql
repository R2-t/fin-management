-- Phase 0.7: exchange rates become scaled integers.
--
-- Rates are stored as `round(rate * 1e9)` so no float is ever persisted or
-- aggregated (see docs/MONEY.md). Conversion of amounts happens once, on the
-- aggregate, with half-away-from-zero rounding.
ALTER TABLE purchase ADD COLUMN IF NOT EXISTS exchange_rate_scaled BIGINT;
ALTER TABLE exchange_rate_cache ADD COLUMN IF NOT EXISTS rate_scaled BIGINT;

UPDATE purchase
   SET exchange_rate_scaled = round(exchange_rate * 1000000000)::bigint
 WHERE exchange_rate IS NOT NULL
   AND exchange_rate_scaled IS NULL;

UPDATE exchange_rate_cache
   SET rate_scaled = round(rate * 1000000000)::bigint
 WHERE rate_scaled IS NULL;

ALTER TABLE exchange_rate_cache ALTER COLUMN rate_scaled SET NOT NULL;

-- The view references `purchase.exchange_rate`, so it must go first.
DROP VIEW IF EXISTS purchase_items;

ALTER TABLE purchase DROP COLUMN IF EXISTS exchange_rate;
ALTER TABLE exchange_rate_cache DROP COLUMN IF EXISTS rate;

-- Recreate the view with integer math. `total_wallet` converts the aggregate
-- once (`sum * rate / 1e9`, rounded), not each line item.
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
           round(
               sum(i.quantity * i.unit_price)::numeric
               * COALESCE(p.exchange_rate_scaled, 1000000000)
               / 1000000000
           )::bigint as total_wallet,
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
