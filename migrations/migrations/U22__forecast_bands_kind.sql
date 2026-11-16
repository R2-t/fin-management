-- The uncertainty bands are persisted like every other forecast, so they can
-- be back-tested against the deterministic answer they are meant to improve
-- (6.7). A new kind rather than overloading `cash_runway`: a band is not a
-- runway, it is a statement about one, and conflating them would make the
-- back-test unable to tell what it was scoring.
ALTER TABLE forecast DROP CONSTRAINT IF EXISTS forecast_kind_check;
ALTER TABLE forecast
    ADD CONSTRAINT forecast_kind_check CHECK (kind IN (
        'budget_overspend', 'cash_runway', 'runway_bands'
    ));
