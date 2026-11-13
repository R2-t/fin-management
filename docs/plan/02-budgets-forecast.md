# Phase 2 — Budgets, income, forecast v1 (Rust, deterministic)

- [~] 2.1 Migrations + CRUD: `budget`, `income_schedule`, `recurring_rule`
- [x] 2.2 Wallet balance: `ledger_entry` (or opening balance + derived) and `wallet_balance`
- [ ] 2.3 Baseline burn rate + weekday-median profile (SQL aggregates)
- [ ] 2.4 Overspend forecast per budget (projected end-of-period vs limit)
- [ ] 2.5 Cash-runway simulation to next payday (integer math, recurring + income)
- [ ] 2.6 `GET /forecast/*` with assumptions list; persist to `forecast` table
- [ ] 2.7 Property tests on simulation

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

The schema for 2.1 also exists (`transfer`, `budget`, `budget_period`,
`income_schedule`, `income_allocation`, `income_receipt`, `recurring_rule`,
`forecast`) but no CRUD is wired yet.

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
