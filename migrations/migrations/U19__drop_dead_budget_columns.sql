-- Resolves the two dead columns the review found in the budget tables.
--
-- Both were decisions that outlived their own reasoning, and both were worse
-- than absent: each was written, stored and returned to the API caller while
-- no code path acted on it.

-- D30 recorded two independent carry flags. ADR-0010 later settled that
-- unspent money is reported beside the limit and never raises it -- Jev 0.82
-- for reporting it separately, 0.00 for raising the limit -- which leaves
-- `rollover_unspent` describing behaviour the product has rejected. The API
-- accepted it from the caller and handed it back, so a caller could set it,
-- see it come back, and nothing differ. A control that changes nothing is
-- worse than no control: it invites the reader to expect an effect. (Jev
-- 0.97 to remove it.)
--
-- `budget_period.unspent_minor` was written as 0 when the period was created
-- and never updated, while the API computed the real figure at read time. The
-- database said 0 and the API said 200 for the same period. The value changes
-- every day as the user spends, so it cannot be meaningfully written once; it
-- is derived when the period is read. (Jev 0.94 to drop it.)

ALTER TABLE budget DROP COLUMN IF EXISTS rollover_unspent;
ALTER TABLE budget_period DROP COLUMN IF EXISTS unspent_minor;
