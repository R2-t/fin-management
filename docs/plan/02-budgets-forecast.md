# Phase 2 — Budgets, income, forecast v1 (Rust, deterministic)

- [x] 2.1 Migrations + CRUD: `budget`, `income_schedule`, `recurring_rule`
- [x] 2.2 Wallet balance: `ledger_entry` (or opening balance + derived) and `wallet_balance`
- [x] 2.3 Baseline burn rate + weekday-median profile (SQL aggregates)
- [x] 2.4 Overspend forecast per budget (projected end-of-period vs limit)
- [~] 2.5 Cash-runway simulation to next payday (integer math, recurring + income)
- [x] 2.6 `GET /forecast/*` with assumptions list; persist to `forecast` table
- [x] 2.7 Property tests on simulation

## Status

**2.2 done.** The ledger is append-only and a Wallet's balance is the sum of its
entries (ADR-0004). A Purchase writes its debit in the same transaction that
records it (ADR-0007), taking its amount from `purchase_items.total_wallet` --
the same source the analytics projection reads, so the balance and the reported
spend cannot drift. An unresolved conversion produces no debit, because there
is no converted amount to charge (ADR-0006).

Endpoints:

```
GET  /wallet/:id/balance   the sum of the wallet's entries
GET  /wallet/:id/ledger    the entries themselves, newest first
POST /wallet/:id/ledger    an opening balance or a manual adjustment
```

**2.5 done.** The runway projects forward from the ledger balance and reports
the trough and the recovery date. It runs forward even when the balance is
already negative (D38). The horizon is 90 days and the response carries its
assumptions (D44, D45).

**2.1 done.** The Budget CRUD is built: a template plus lazily materialised
periods, with the carry rules from ADR-0009 and ADR-0010.

```
GET    /budget              the user's budgets
POST   /budget              declares a budget
GET    /budget/:id/status   the current period and how it is going
PUT    /budget/:id/period   sets a period's limit
DELETE /budget/:id          deletes a budget
```
The recurring matcher is built and shared (`application::recurring::matcher`),
because the burn rate needs it now and Phase 4.4's unintended-recurring
detection will need it again.

**The Runway design is settled** (grilling session; ADR-0012, decisions
D38-D45). The simulation is not written; `income_schedule` needs its CRUD
first, because a Runway without scheduled income cannot report a recovery
date.

**2.3 done.** The burn rate is the discretionary spending pace. The income
CRUD is built, so the Runway has scheduled income to project. The recurring
rules CRUD and the burn rate endpoint are built.

Endpoints:

```
GET    /income             the user's income schedules
POST   /income             declares an expectation
POST   /income/:id/receive records that it arrived; credits the wallets
DELETE /income/:id         deletes the expectation
GET    /recurring          the user's recurring commitments
POST   /recurring          declares a commitment
POST   /recurring/:id/cancel  stops it being forecast
DELETE /recurring/:id      deletes it
GET    /analytics/burn-rate    the discretionary spending pace
GET    /analytics/runway/:wallet_id   the projection: trough and recovery date
GET    /forecast               the user's persisted forecasts, newest first
GET    /forecast/budget/:budget_id/overspend   the overspend projection
```

**2.4, 2.6 and 2.7 done.** The overspend projection is persisted to the
`forecast` table with its assumptions and its `model_version`, so a forecast
can be compared and back-tested later. The simulation's arithmetic was
extracted into a pure function (`application::runway::projection`) so the
property tests exercise the same code the service runs.

## Notes

- **A correction supersedes the prior debit rather than editing it.** The
  ledger is append-only, so an edited amount, a corrected rate, an added or
  removed item, and a deleted purchase all close the old entry and write a
  replacement. The balance sums active entries only; the history of
  corrections is preserved. The unique index covers only *active* purchase
  debits, so a replayed event cannot double-charge a wallet while superseded
  entries stay in the ledger.
- **The debit amount is read from the `purchase_items` view**, not recomputed
  from the items. One rounding rule, one source: the view already applies the
  half-away-from-zero rule, and the projection reads the same column.
- **A ledger entry cannot be in a currency the wallet does not hold.** That
  would make the balance a sum across currencies (ADR-0002). The check is at
  the API boundary, and the wallet's currency is authoritative.
- **A wallet that is not the caller's returns not-found**, not an empty list.
  An empty list reads as "this wallet has no entries" when it actually means
  "not yours". This was a real defect the integration test caught.
- **The ledger listing is bounded** (default 50, max 500) and the bounds live
  in the application service, not in the handler.
- **Budget periods are materialised lazily**, on read. Nothing raises a
  period's limit automatically: unspent money is reported beside the limit,
  not added to it, because the user may be deliberately saving it (ADR-0010).
- **The recurring matcher is deliberately conservative.** A false positive
  removes spending from the burn rate and makes the forecast optimistic; a
  false negative leaves it in and charges the subscription twice. Under-
  matching is the safer error, so a purchase at a different merchant never
  matches, and one purchase matches at most one rule.
- **Month arithmetic clamps to the last day of the month**, and the anchor day
  is preserved across the clamping: a rule on the 31st fires on the 28th in
  February and returns to the 31st in March.
- **The burn rate measures over the history the user actually has**, which
  starts at their first purchase, not at the start of the 30-day window.
  Counting the days before they started recording as zero-spend days would
  understate their pace.
- **The weekday profile uses the median, not the mean.** One large purchase
  should not define a weekday's pace. The median of an even count is the
  lower of the two middle values.
- **A test that projects must hold the projection lock.** The projector claims
  work from the *global* outbox, so an unlocked test can have its rows claimed
  by another test's drain. Two of the burn-rate tests failed this way before
  the lock was added.
- **A budget's period is summed from the DAILY series**, not a monthly
  bucket. A budget's period does not align with the calendar month -- a reset
  day of 15 runs from the 15th to the 14th -- so a monthly aggregate would
  count the wrong days.
- **A reset day of -1 means the period ENDS on the last day of its month**,
  which is the same month a reset day of 1 covers but is expressed as an end,
  because that is how a month-end payday reads.
- **The forecast's invariants are property-tested**, not just case-tested:
  the trough is never above the starting balance, the ending balance is the
  arithmetic, a recovery date exists exactly when the balance dipped and came
  back, and the projection never creates money without income.
- **`forecast.horizon_days` is BIGINT**, not INT: the Rust side carries day
  counts as i64, and the mismatch was a runtime error rather than a compile
  one.
