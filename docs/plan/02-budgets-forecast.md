# Phase 2 — Budgets, income, forecast v1 (Rust, deterministic)
- [ ] 2.1 Migrations + CRUD: `budget`, `income_schedule`, `recurring_rule`
- [ ] 2.2 Wallet balance: `ledger_entry` (or opening balance + derived) and `wallet_balance`
- [ ] 2.3 Baseline burn rate + weekday-median profile (SQL aggregates)
- [ ] 2.4 Overspend forecast per budget (projected end-of-period vs limit)
- [ ] 2.5 Cash-runway simulation to next payday (integer math, recurring + income)
- [ ] 2.6 `GET /forecast/*` with assumptions list; persist to `forecast` table
- [ ] 2.7 Property tests on simulation
