-- The app binds exchange_rate (and cached rates) as f64, which tokio-postgres
-- only maps to DOUBLE PRECISION — not NUMERIC. Convert the rate columns.
-- The app binds exchange_rate (and cached rates) as f64, which tokio-postgres
-- only maps to DOUBLE PRECISION — not NUMERIC. Convert the rate columns.
-- `purchase_items` depends on purchase.exchange_rate, so recreate it after.
DROP VIEW IF EXISTS purchase_items;

ALTER TABLE purchase
    ALTER COLUMN exchange_rate TYPE DOUBLE PRECISION;

ALTER TABLE exchange_rate_cache
    ALTER COLUMN rate TYPE DOUBLE PRECISION;

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