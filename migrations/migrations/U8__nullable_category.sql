-- Phase 0.5: a purchase may be uncategorized until the classifier assigns one.
-- `classification_status` records where the category came from.
ALTER TABLE purchase ALTER COLUMN category_id DROP NOT NULL;

ALTER TABLE purchase
    ADD COLUMN IF NOT EXISTS classification_status TEXT NOT NULL DEFAULT 'unclassified';

-- The view joined `category` with an inner JOIN. With a nullable `category_id`
-- that would silently drop uncategorized purchases from the view.
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
        LEFT JOIN category c on p.category_id = c.id
        JOIN wallet w on p.wallet_id = w.id
    WHERE p.deleted_at IS NULL
    GROUP BY p.id, c.name, w.currency;
