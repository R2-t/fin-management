-- The trained model's weights live with its registry row.
--
-- `model_registry` recorded a version, metrics and an active flag, but nothing
-- held the model itself, so "active" could not be loaded by the classifier. The
-- artifact is small on purpose: a centroid classifier over character n-grams is
-- a vocabulary and a weight vector per category, a few hundred KB at this
-- scale, and keeping it in the row means promotion and rollback are one UPDATE
-- rather than a file that has to be shipped alongside the database.
ALTER TABLE model_registry
    ADD COLUMN IF NOT EXISTS artifact JSONB;

COMMENT ON COLUMN model_registry.artifact IS
    'The serialized model (vocabulary + per-category weights). NULL for rows registered before the column existed; those are not loadable and never active.';

-- Only a row that can be loaded may be active.
CREATE UNIQUE INDEX IF NOT EXISTS model_registry_loadable_active_idx
    ON model_registry (active)
    WHERE active AND artifact IS NOT NULL;
