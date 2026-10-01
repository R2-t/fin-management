# Phase 1 — Historical trends (SQL only)
- [ ] 1.1 Migration: `purchase_fact` hypertable + indexes (user_id first)
- [ ] 1.2 Projector task (claim, coalesce, rebuild, retry, dead state) + lag metric  (dep 0.7, 0.8)
- [ ] 1.3 Backfill command
- [ ] 1.4 Query layer: `time_bucket` with user timezone, per base currency
- [ ] 1.5 Endpoints: category breakdown (custom range), MoM growth, rolling avg (7/30/90d); include `as_of`
- [ ] 1.6 Tests: edit date, delete, currency change, multi-user isolation
- [ ] 1.7 (Later, only if slow) daily CAGG / rollup
