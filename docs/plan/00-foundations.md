# Phase 0 — Foundations (blocks everything)

Includes the analytics substrate (`purchase_fact` + outbox + Timescale), per decision.
Order below is dependency order. Sizes: S ≤1d · M 2–4d · L 1–2w.

## Decisions taken
- `purchase.category_id` becomes **nullable**; Python classification may override it later
  (user decision, overrides Jev's "system Uncategorized category").
- TimescaleDB adopted **now** (user decision; Jev was 0.54 to defer).
- `purchase_fact` lives in **Phase 0**, not Phase 1.
- `user_profile.base_currency` is the default; `wallet.currency` may override (Jev 0.77).
- Exchange rate migrates to **scaled BIGINT** (Jev 0.88).

## Tasks
- [x] 0.1 Compose: pin `timescale/timescaledb:2.23.0-pg18`; new migration with
      `CREATE EXTENSION IF NOT EXISTS timescaledb`; record `SHOW timescaledb.license` in the README (S)
- [x] 0.2 Money & rounding conventions doc: scaled rates, one rounding rule, reconciliation criteria (S)
- [x] 0.3 `user_profile(user_id, timezone, base_currency, locale)` + `GET`/`PUT` endpoints,
      default `UTC`; seed rows for user ids already present in `wallet`/`purchase` (S–M) — done:
  `PUT` is a partial update; IANA name validated with `chrono-tz` (case-insensitive),
  offsets like `+05:30` are rejected even though Postgres accepts them; `locale` cannot be cleared
- [x] 0.4 Wallet create/update accept + validate ISO-4217 `currency`; expose it in wallet DTOs (S)
      (repo is P1-6 in `docs/reports/api-review.md`; today every API wallet is `'USD'`)
- [x] 0.5 Make `category_id` nullable and add `classification_status`:
      migration `DROP NOT NULL`; **`purchase_items` view `JOIN category` → `LEFT JOIN`** (otherwise
      uncategorized rows disappear); Rust `i32` → `Option<i32>` in model, `row_to_purchase`,
      `CreatePurchaseCommand`, handler `RequestBody`/DTO; keep the `user_id, category_id` index (M)
- [x] 0.6 `merchant` + `merchant_alias`; nullable `purchase.merchant_id`; keep `store` text;
      backfill merchants from distinct `purchase.store` per user (M)
- [ ] 0.7 `exchange_rate` → scaled BIGINT (`rate * 1e9`) on `purchase` and `exchange_rate_cache`;
      recreate the view with `total_wallet` from integer math + one rounding rule (M)
- [ ] 0.8 `analytics_outbox` + `analytics_job` tables (M)
- [ ] 0.9 Write the outbox row in the same transaction as purchase/item/wallet mutations,
      mirroring the existing `insert_history` pattern (+ tests) (M)
- [ ] 0.10 `purchase_fact` hypertable + indexes (`user_id` first), partitioned by `occurred_at`;
      append-only, no FKs; never hypertable `purchase` (M)
- [ ] 0.11 Projector task (claim with `FOR UPDATE SKIP LOCKED`, coalesce by purchase, delete+insert
      rebuild, retry/dead state, lag metric) + backfill command + reconciliation tests (L)
- [ ] 0.12 API: pagination envelope with bounds, stable ordering, expose date-range/category/wallet
      filters, and stop silently ignoring the `store` filter (M)

## Notes
- 0.1 done: image `timescale/timescaledb:2.23.0-pg18`; extension 2.23.0; `SHOW timescaledb.license`
  = `timescale` (TSL), so continuous aggregates and compression are available.
- 0.4 done: shared `application::currency::normalize_currency`; `wallet.currency` settable and
  echoed in wallet DTOs. Default is `USD` until 0.3 supplies `user_profile.base_currency`.
  `PUT /wallet/:id` still requires `name` (pre-existing PUT shape).
- 0.5 and 0.6 both touch `purchase`; batch into one migration series where practical.
- 0.10 must follow 0.1 (extension), 0.5 (category), 0.6 (merchant) and 0.7 (rate) — it consumes all three.
- This is a large phase by design; 0.3–0.7 are independently shippable.
