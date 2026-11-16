-- Phase 0 hardening: conversion must account for differing decimal places.
--
-- Amounts are minor units, so converting between currencies needs BOTH the
-- exchange rate (major units of one currency to major units of the other) and the
-- ratio of their minor-unit exponents:
--
--   amount_minor_target = amount_minor_source * rate * 10^(target_exp - source_exp)
--
-- The view previously applied only the rate, so a USD purchase into a JPY wallet
-- came out 100x too small (JPY has no decimals, USD has two). Same-exponent pairs
-- are unaffected (10^0 = 1), which is why this went unnoticed for USD/EUR.
--
-- The `currency` reference table added in U14 now has its SQL-side consumer.
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
        LEFT JOIN currency cs on cs.code = p.currency
        LEFT JOIN currency cw on cw.code = w.currency
    WHERE p.deleted_at IS NULL
    GROUP BY p.id, c.name, w.currency, cs.minor_unit, cw.minor_unit;

-- Facts materialise `total_wallet`, so rows written before this fix carry the
-- old, exponent-blind amounts. They must be reprojected:
--   cargo run -- --backfill-facts
COMMENT ON TABLE purchase_fact IS
    'Derived analytics projection. Reproject with --backfill-facts after any change to the purchase_items view.';
