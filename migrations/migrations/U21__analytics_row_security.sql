-- The sidecar's read access is scoped to one user per connection (D4).
--
-- The `analytics` role could read every user's merchants, aliases, purchases
-- and facts. The scoping lived in the queries alone, so a forgotten WHERE
-- clause was not a bug but a leak, and nothing in the database would have
-- noticed.
--
-- Row-level security makes the boundary a property of the database instead.
-- The sidecar states which user it is acting for with
-- `set_config('app.user_id', ...)` and can then only see that user's rows.
--
-- The application is unaffected: it connects as the tables' owner, and an
-- owner bypasses row-level security unless it is forced. Its own queries
-- already filter by user_id.

-- The policy below is named the same on every table. A policy name only has
-- to be unique per table, and one name for one idea keeps them findable.

ALTER TABLE merchant ENABLE ROW LEVEL SECURITY;
CREATE POLICY analytics_tenant ON merchant
    USING (user_id = current_setting('app.user_id', true)::uuid);

-- Aliases have no user_id of their own; they belong to a merchant, and the
-- merchant's policy filters the subquery for a role that is subject to it.
ALTER TABLE merchant_alias ENABLE ROW LEVEL SECURITY;
CREATE POLICY analytics_tenant ON merchant_alias
    USING (EXISTS (
        SELECT 1 FROM merchant m
         WHERE m.id = merchant_alias.merchant_id
    ));

ALTER TABLE purchase ENABLE ROW LEVEL SECURITY;
CREATE POLICY analytics_tenant ON purchase
    USING (user_id = current_setting('app.user_id', true)::uuid);

ALTER TABLE purchase_fact ENABLE ROW LEVEL SECURITY;
CREATE POLICY analytics_tenant ON purchase_fact
    USING (user_id = current_setting('app.user_id', true)::uuid);

-- System categories belong to no user and are shared, so every tenant sees
-- them alongside their own.
ALTER TABLE category ENABLE ROW LEVEL SECURITY;
CREATE POLICY analytics_tenant ON category
    USING (user_id IS NULL OR user_id = current_setting('app.user_id', true)::uuid);
