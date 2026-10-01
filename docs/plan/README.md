# Analytics & Forecasting Plan

Multi-user app. Status legend: `[ ]` todo · `[~]` in progress · `[x]` done.
Source reports: `docs/reports/`. Decisions: `docs/plan/DECISIONS.md`.

| Phase | File | Depends on | Delivers |
|---|---|---|---|
| 0 | [00-foundations.md](00-foundations.md) | – | profile, wallet currency, merchant, outbox/job tables, Timescale image |
| 1 | [01-trends.md](01-trends.md) | 0 | fact projection + MoM / rolling avg / category breakdown APIs |
| 2 | [02-budgets-forecast.md](02-budgets-forecast.md) | 0, 1 | budgets, income, recurring, ledger, deterministic forecast |
| 3 | [03-python-sidecar.md](03-python-sidecar.md) | 0 (1 for training data) | FastAPI service, merchant cleaning, categorization, probabilistic forecast |
| 4 | [04-anomalies.md](04-anomalies.md) | 1, 3 | duplicates, outliers, unintended recurring |
| 5 | [05-hardening.md](05-hardening.md) | all | privacy, observability, load, retention |

Order: 0 → 1 → 2 → 3 → 4 → 5 (3 can start after 0 in parallel with 1/2).
