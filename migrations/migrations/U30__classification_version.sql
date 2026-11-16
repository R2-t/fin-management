-- The version a suggestion was computed at (D65).
--
-- The worker already refuses a job whose version is stale, but the *applier*
-- did not know the version at all: it applied any prediction to any purchase
-- whose category was NULL, at whatever version the purchase had reached by
-- then. A prediction computed at v3 could be applied at v9 -- including after
-- the user cleared the category or changed the merchant, which is exactly what
-- the version check exists to prevent.
--
-- NULL means "recorded before this column existed": those rows are not applied,
-- because there is no version to compare against and guessing is worse than
-- leaving them.
ALTER TABLE purchase_classification
    ADD COLUMN IF NOT EXISTS enqueued_version INTEGER;

COMMENT ON COLUMN purchase_classification.enqueued_version IS
    'The purchase version the suggestion was computed at (D65). The applier only applies a suggestion whose version still matches.';
