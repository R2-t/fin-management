-- Phase 0.6: merchant identity.
--
-- `purchase.store` stays as the raw text the user typed. `merchant` is the
-- cleaned/normalised entity used for categorisation, dedup and recurring-charge
-- detection; `merchant_alias` maps raw variants onto one merchant.
CREATE TABLE IF NOT EXISTS merchant (
    id              UUID NOT NULL PRIMARY KEY,
    user_id         UUID NOT NULL,
    name            TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at      TIMESTAMPTZ
);

-- One live merchant per (user, normalised name).
CREATE UNIQUE INDEX IF NOT EXISTS merchant_user_normalized_idx
    ON merchant (user_id, normalized_name)
    WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS merchant_alias (
    id               UUID NOT NULL PRIMARY KEY,
    merchant_id      UUID NOT NULL REFERENCES merchant (id) ON DELETE CASCADE,
    alias            TEXT NOT NULL,
    normalized_alias TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (merchant_id, normalized_alias)
);

ALTER TABLE purchase
    ADD COLUMN IF NOT EXISTS merchant_id UUID REFERENCES merchant (id);

CREATE INDEX IF NOT EXISTS idx_purchase_merchant ON purchase (user_id, merchant_id);

-- Backfill: one merchant per distinct normalised store string per user, then
-- link the purchases. Normalisation here is only lower+trim; richer cleaning
-- (payment-processor prefixes, trailing reference numbers) belongs to Phase 3.
INSERT INTO merchant (id, user_id, name, normalized_name)
SELECT gen_random_uuid(), p.user_id, min(p.store), lower(btrim(p.store))
FROM purchase p
WHERE p.store IS NOT NULL AND btrim(p.store) <> ''
GROUP BY p.user_id, lower(btrim(p.store))
ON CONFLICT DO NOTHING;

UPDATE purchase p
SET merchant_id = m.id
FROM merchant m
WHERE p.merchant_id IS NULL
  AND p.store IS NOT NULL
  AND m.user_id = p.user_id
  AND m.normalized_name = lower(btrim(p.store));
