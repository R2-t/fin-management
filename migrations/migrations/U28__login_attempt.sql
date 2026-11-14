-- Login attempts: one row per try, successful or not, so repeated failures can
-- be counted against the email and the address that made them (D73).
--
-- Recording successes too is deliberate: a successful login clears the email's
-- failure rows, so a user who mistypes twice and then gets it right is not
-- left in a window that counts against them.
CREATE TABLE IF NOT EXISTS login_attempt (
    id           BIGSERIAL PRIMARY KEY,
    email        CITEXT,
    client_ip    INET,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    succeeded    BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_login_attempt_email
    ON login_attempt (email, attempted_at);
CREATE INDEX IF NOT EXISTS idx_login_attempt_ip
    ON login_attempt (client_ip, attempted_at);

REVOKE ALL ON login_attempt FROM analytics;
