# Phase 1 — Historical trends (SQL only)

Depends on Phase 0 (fact projection now lives there).

- [ ] 1.1 Query layer: `time_bucket` with the user's timezone, grouped per base currency
- [ ] 1.2 Category breakdown + totals over a custom range
- [ ] 1.3 Month-over-month growth (window `lag`)
- [ ] 1.4 Rolling averages (7 / 30 / 90 d)
- [ ] 1.5 `GET /analytics/trends` with range/category/wallet/currency filters; return `as_of`
      (facts lag the outbox by ~1s)
- [ ] 1.6 Serialize money as i64/string, never f64; contract tests
- [ ] 1.7 Tests: edit date, delete, currency change, uncategorized rows, multi-user isolation
- [ ] 1.8 (Later, only if slow) daily CAGG / rollup
