-- At most one open job per purchase (review finding, D79).
--
-- The enqueue used `INSERT ... WHERE NOT EXISTS`, which two concurrent
-- statements both pass under READ COMMITTED: both insert, the worker runs the
-- same job twice, and the enqueue scanned the table on every create. A partial
-- unique index makes the database enforce it, and turns the guard into
-- `ON CONFLICT DO NOTHING`.
--
-- Partial, because a purchase legitimately gets a new job later: only rows that
-- are still open compete.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_analytics_job_open_purchase
    ON analytics_job (kind, (payload->>'purchase_id'))
    WHERE status IN ('pending', 'running')
      AND payload ? 'purchase_id';
