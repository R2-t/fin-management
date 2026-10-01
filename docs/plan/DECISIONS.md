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

## Outbox design (D2)
- `analytics_outbox(id, user_id, aggregate_type, aggregate_id, aggregate_version, created_at, processed_at, attempts, last_error)`; written in the same tx as the OLTP mutation.
- IDs only; projector re-reads current state → idempotent, order-insensitive, coalescing.
- Projector: Rust task, `FOR UPDATE SKIP LOCKED` batches, LISTEN/NOTIFY + poll, per-user batch cap.
- Rebuild = delete by `purchase_id` + insert (partition key includes `occurred_at`).
- Same projector enqueues `analytics_job` rows for Python work.
- Attempts capped → dead state; OTel gauge for lag; backfill command; purge processed after ~7d.
- Responses carry `as_of`; short windows may query OLTP live.
