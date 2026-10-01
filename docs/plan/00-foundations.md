# Phase 0 — Foundations
- [ ] 0.1 Compose: switch to `timescale/timescaledb:<pin>-pg18`; verify license/extension loads
- [ ] 0.2 Migration: `user_profile(user_id, timezone, base_currency)` + API get/put (default tz UTC)
- [ ] 0.3 Wallet create/update accept + validate ISO-4217 `currency` (api-review P1-6)
- [ ] 0.4 Exchange rate: add scaled-bigint rate column, dual-read, stop using f64 for totals
- [ ] 0.5 Migration: `merchant`, `merchant_alias`; `purchase.merchant_id` nullable (store text kept)
- [ ] 0.6 Decide `Uncategorized` system category vs nullable `category_id`; add `classification_status`
- [ ] 0.7 Migration: `analytics_outbox`, `analytics_job`
- [ ] 0.8 Repo layer: write outbox row in same tx as purchase/item/wallet mutations (+ tests)
- [ ] 0.9 API pagination/filters (date range, category, wallet) per api-review
