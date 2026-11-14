-- The sidecar's worker polls the job queue, claims a job, and marks it done
-- or failed.
--
-- Writing the queue is not writing the user's data: a job row is the worker's
-- own bookkeeping, and the user_id on it is the scope the worker must state
-- before touching anything else.
--
-- No INSERT: jobs are enqueued by the application, which owns the write path;
-- the worker only claims and completes them.
GRANT SELECT, UPDATE ON analytics_job TO analytics;
