-- The policy's guard was not NULL-safe.
--
-- `current_setting('app.user_id', true)` returns an EMPTY STRING, not NULL,
-- when the setting has never been set on the connection. Casting '' to uuid
-- is an error, so a query from a connection that had not stated a user failed
-- with "invalid input syntax for type uuid" instead of returning nothing --
-- the opposite of the boundary the policy was meant to enforce.
--
-- A connection that has stated no user must see no rows, not error.

DROP POLICY IF EXISTS analytics_tenant ON merchant;
CREATE POLICY analytics_tenant ON merchant
    USING (user_id::text = NULLIF(current_setting('app.user_id', true), ''));

DROP POLICY IF EXISTS analytics_tenant ON merchant_alias;
CREATE POLICY analytics_tenant ON merchant_alias
    USING (EXISTS (
        SELECT 1 FROM merchant m
         WHERE m.id = merchant_alias.merchant_id
    ));

DROP POLICY IF EXISTS analytics_tenant ON purchase;
CREATE POLICY analytics_tenant ON purchase
    USING (user_id::text = NULLIF(current_setting('app.user_id', true), ''));

DROP POLICY IF EXISTS analytics_tenant ON purchase_fact;
CREATE POLICY analytics_tenant ON purchase_fact
    USING (user_id::text = NULLIF(current_setting('app.user_id', true), ''));

DROP POLICY IF EXISTS analytics_tenant ON category;
CREATE POLICY analytics_tenant ON category
    USING (user_id IS NULL
           OR user_id::text = NULLIF(current_setting('app.user_id', true), ''));
