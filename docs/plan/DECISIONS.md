# Design Decisions (checked with Jev `jev-latest`, probabilities in brackets)

| # | Decision | Choice |
|---|---|---|
| D1 | Analytics store | Append-only `purchase_fact` hypertable; never hypertable `purchase` (0.97) |
| D2 | Fact population | Transactional outbox + Rust projector, not dual-write (0.96) |
| D3 | Rust↔Python | Sync HTTP for inference + Postgres `analytics_job` queue for batch; no broker (0.99) |
| D4 | Python data access | Read-only, user-scoped DB role; writes only its own tables (0.99) |
| D5 | Forecast split | Deterministic runway/burn in Rust (i64); Python adds probabilistic bands (0.99) |
| D6 | Order | Foundations → trends → budgets/forecast → ML → anomalies (1.0) |
| D7 | Timescale timing | Adopt image early, but CAGGs only when needed (adopt-now 0.20) |
| D8 | Prophet | Not in first forecast release; robust baseline first (0.26) |
| D9 | Tenancy | Multi-user: `user_id` on every table/query; global model + per-user merchant aliases |
| D10 | Timescale timing | **Adopt now** (user decision; Jev 0.54 to defer) |
| D11 | Uncategorized purchases | `purchase.category_id` **nullable** + `classification_status`; Python may override later (user decision, overriding Jev's system-category 0.94) |
| D12 | Reporting currency | `user_profile.base_currency` default, `wallet.currency` override (Jev 0.77) |
| D13 | Exchange rate storage | Scaled BIGINT (`rate * 1e9`), one rounding rule (Jev 0.88) |
| D14 | Phase 0 scope | Single phase, includes `purchase_fact` + projector (user decision; Jev 1.0 for splitting) |

## Consequence of D11
`purchase_items` joins `category` with an inner JOIN, so uncategorized purchases would disappear
from the view. It must become a `LEFT JOIN`, and `category_id` must become `Option<i32>` in the model,
repository row mapping, commands, handler DTOs and `CreatePurchaseCommand`.

## Outbox design (D2)
- `analytics_outbox(id, user_id, aggregate_type, aggregate_id, aggregate_version, created_at, processed_at, attempts, last_error)`; written in the same tx as the OLTP mutation.
- IDs only; projector re-reads current state → idempotent, order-insensitive, coalescing.
- Projector: Rust task, `FOR UPDATE SKIP LOCKED` batches, LISTEN/NOTIFY + poll, per-user batch cap.
- Rebuild = delete by `purchase_id` + insert (partition key includes `occurred_at`).
- Same projector enqueues `analytics_job` rows for Python work.
- Attempts capped → dead state; OTel gauge for lag; backfill command; purge processed after ~7d.
- Responses carry `as_of`; short windows may query OLTP live.
