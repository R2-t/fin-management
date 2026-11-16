-- The cast belongs on the SETTING, not on the column.
--
-- U24's policy qual cast the indexed column to text
-- (`user_id::text = NULLIF(...)`), so the policy could not use
-- `idx_purchase_user_category` or `idx_purchase_user_date`, and every
-- tenant-scoped read seq-scanned `purchase` (review finding 8). Casting the
-- setting to uuid instead is equally NULL-safe and keeps the index.
DROP POLICY IF EXISTS analytics_tenant ON merchant;
CREATE POLICY analytics_tenant ON merchant
    USING (user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);

DROP POLICY IF EXISTS analytics_tenant ON merchant_alias;
CREATE POLICY analytics_tenant ON merchant_alias
    USING (EXISTS (
        SELECT 1 FROM merchant m
         WHERE m.id = merchant_alias.merchant_id
    ));

DROP POLICY IF EXISTS analytics_tenant ON purchase;
CREATE POLICY analytics_tenant ON purchase
    USING (user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);

DROP POLICY IF EXISTS analytics_tenant ON purchase_fact;
CREATE POLICY analytics_tenant ON purchase_fact
    USING (user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);

DROP POLICY IF EXISTS analytics_tenant ON category;
CREATE POLICY analytics_tenant ON category
    USING (user_id IS NULL
           OR user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
