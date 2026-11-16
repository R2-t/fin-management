-- Every analytics job row carries the credential that names its tenant (D72).
--
-- The worker takes user_id from the verified claim, never from `payload`, so a
-- compromised sidecar cannot pick a tenant by editing what it reads. The token
-- is short-lived: a job whose token has expired fails terminally rather than
-- running unscoped, because a job that ran as the wrong tenant cannot be undone.
ALTER TABLE analytics_job
    ADD COLUMN IF NOT EXISTS auth_token TEXT;

COMMENT ON COLUMN analytics_job.auth_token IS
    'Short-lived PASETO service token naming the job tenant (D72). Verified by the worker; the claim decides app.user_id, never the payload.';
