# Analytics & Forecasting Plan

Multi-user app. Status legend: `[ ]` todo · `[~]` in progress · `[x]` done.
Source reports: `docs/reports/`. Decisions: `docs/plan/DECISIONS.md`.

| Phase | File | Depends on | Delivers |
|---|---|---|---|
| 0 | [00-foundations.md](00-foundations.md) | – | Timescale, profile, wallet currency, nullable category, merchant, outbox, `purchase_fact` + projector |
| 1 | [01-trends.md](01-trends.md) | 0 | fact projection + MoM / rolling avg / category breakdown APIs |
| 2 | [02-budgets-forecast.md](02-budgets-forecast.md) | 0, 1 | budgets, income, recurring, ledger, deterministic forecast |
| 3 | [03-python-sidecar.md](03-python-sidecar.md) | 0 (1 for training data) | FastAPI service, merchant cleaning, categorization, probabilistic forecast |
| 4 | [04-anomalies.md](04-anomalies.md) | 1, 3 | duplicates, outliers, unintended recurring |
| 5 | [05-hardening.md](05-hardening.md) | all | privacy, observability, load, retention |
| 6 | [06-precision.md](06-precision.md) | 2, 3.8 | weekday-profile forecasts, bootstrap bands, back-testing |

Order: 0 → 1 → 2 → 3 → 4 → 5, with 6 (forecast precision) after 3.8 and parallel to 4/5.
Phase 0 is deliberately large and includes the fact projection; its items 0.3–0.7 are independently shippable.
