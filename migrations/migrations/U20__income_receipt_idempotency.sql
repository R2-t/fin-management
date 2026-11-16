-- Income receipts are idempotent, and the credit points back at the receipt.
--
-- The endpoint that records an arrival could be called twice -- a
-- double-click, a client retry after a timeout -- and each call wrote a
-- receipt AND credited the wallets again. Nothing in the schema prevented it,
-- and because the credit carried no reference to the receipt that caused it,
-- a duplicate could not even be found afterwards.
--
-- An arrival is identified by its schedule plus the CALENDAR DATE it arrived
-- on. One arrival per schedule per day: a retry on the same day is the same
-- arrival, and recording it on a different day is a different arrival. The
-- date rather than the exact instant, because a retry that omits the
-- timestamp defaults to now and would not match the first call.
--
-- The credit now references its receipt, so a credit can be traced to its
-- cause and a receipt can be reversed later by superseding its entries the
-- way a purchase correction is.

-- The occurrence day, in the user's calendar (D25). It is always written
-- explicitly, because the user might be east or west of UTC and the day the
-- money arrived is their day, not the server's.
ALTER TABLE income_receipt
    ADD COLUMN IF NOT EXISTS occurred_on DATE;

-- The existing rows were all received today.
UPDATE income_receipt
   SET occurred_on = DATE_TRUNC('day', occurred_at)
 WHERE occurred_on IS NULL;

ALTER TABLE income_receipt
    ALTER COLUMN occurred_on SET NOT NULL;

-- One arrival per schedule per day. A repeat is refused by the database
-- rather than by the application getting it right.
CREATE UNIQUE INDEX IF NOT EXISTS income_receipt_occurrence_unique
    ON income_receipt (income_schedule_id, occurred_on);

-- The credit points back at the receipt that produced it. `superseded_at`
-- reuses the ledger's existing correction mechanism: reversing a receipt
-- closes its entries rather than deleting them.
ALTER TABLE ledger_entry
    ADD COLUMN IF NOT EXISTS income_receipt_id UUID REFERENCES income_receipt(id);

CREATE INDEX IF NOT EXISTS ledger_entry_income_receipt_idx
    ON ledger_entry (income_receipt_id)
    WHERE income_receipt_id IS NOT NULL;
