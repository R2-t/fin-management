# Phase 1 — Historical trends (SQL only)

Depends on Phase 0 (fact projection now lives there).

- [x] 1.1 Query layer: `time_bucket` with the user's timezone, grouped per base currency
- [x] 1.2 Category breakdown + totals over a custom range
- [x] 1.3 Month-over-month growth (window `lag`)
- [x] 1.4 Rolling averages (7 / 30 / 90 d)
- [x] 1.5 `GET /analytics/trends` with range/category/wallet/currency filters; return `as_of`
      (facts lag the outbox by ~1s)
- [x] 1.6 Serialize money as i64/string, never f64; contract tests
- [x] 1.7 Tests: edit date, delete, currency change, uncategorized rows, multi-user isolation
- [ ] 1.8 (Later, only if slow) daily CAGG / rollup — not needed; the fact table is
      already a hypertable and the queries are indexed by (user_id, occurred_at)

## Notes
- Endpoints: `GET /analytics/trends?bucket=day|week|month&from=&to=&wallet_id=&category_id=`
  and `GET /analytics/rolling?window_days=1..365&from=&to=...`. Both return `as_of`
  and `projection_delay_seconds`, so a caller can tell how fresh the figures are.
- Bucketing is query-time in `user_profile.timezone` (D25); bucket starts are UTC
  instants and must be read back in that timezone to be labelled. A test asserts
  the Madrid October boundary to pin this.
- Totals are per-currency integers from `amount_base_minor`; unresolved conversions
  contribute nothing and are counted separately. Rolling averages are computed in
  the service with the project's single rounding rule, so no money rounding lives
  in a second language.
- Found while building this: `purchase_date` was NOT editable, so a purchase could
  not be corrected into another period even though the projection was designed for
  it (delete+insert exists because a date change moves the partition key). The
  update path now accepts it and the audit trail records the move.
