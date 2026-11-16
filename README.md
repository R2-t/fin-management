# financial-management

A record-your-spending app: record what you bought, against wallets, and get
trustworthy figures back — trends, budgets, forecasts, and runway. Built for
one owner, so every decision favours correctness over scale.

## The rules everything else obeys

- **Money is `i64` minor units.** Never a float. Conversions scale by integer
  exponents from ISO-4217 (`src/application/currency.rs`).
- **Never sum across currencies.** Every wallet is one currency; analytics
  report per wallet and per currency (ADR-0002).
- **Nothing is invented.** A purchase with an unknown conversion is recorded as
  *unresolved*, not treated as parity (ADR-0006). A forecast without history
  says so instead of projecting from four days.
- **Tenancy is explicit.** Every query carries `user_id`; the sidecar's role is
  row-level-security-scoped and cannot write a purchase (D4).
- **The ledger is append-only.** Corrections supersede; history is never
  rewritten.

## Architecture

```text
                    ┌──────────────────────────────┐
  HTTP ────────────▶│  Rust API (axum)             │
                    │  sessions · PASETO · tenancy │
                    └──────┬───────────────────────┘
                           │
              ┌────────────┴────────────┐
              │                         │
     ┌────────▼────────┐       ┌────────▼─────────┐
     │ Postgres        │◀──────│ Python sidecar   │
     │ TimescaleDB     │  HTTP │ FastAPI + worker │
     │ append-only     │  jobs │ (FastAPI + job   │
     │ ledger, outbox, │       │  worker, pg job  │
     │ job queue       │       │  queue)          │
     └─────────────────┘       └──────────────────┘
```

- **Rust** owns every write to money. The Python sidecar reads tenant-scoped
  data, computes suggestions and statistics, and writes only analytics-owned
  tables.
- **Calls from Rust to the sidecar carry a PASETO service token** naming the
  tenant; the sidecar verifies it and takes the tenant from the claim, never
  from a request body (D68/D76).
- **Job queue:** `analytics_job` rows, claimed with `FOR UPDATE SKIP LOCKED`
  by the Python worker.

## What it does

| Area | What exists |
|---|---|
| Recording | Wallets, purchases with line items, explicit transfers, income schedules |
| Money | ISO-4217 minor units, unresolved-rate handling with completion worker |
| Derivations | Ledger balances, purchase facts, trends, rolling averages, category breakdowns |
| Planning | Budgets as limits with lazy periods and carry-overspend (D30), recurring rules, burn rate (weekday-median profile), runway with bootstrap bands |
| Suggestions | Merchant-history rule (applies) + cross-user model (proposes only, D84); corrections feed the learning loop |
| Auth | Session login (hand-rolled, ADR-0013), PASETO for machine clients, `AUTH_SKIP_VERIFY` refused in release |
| Groups | Account groups (a filter over wallets, never a total) |

## Quickstart

```bash
# 1. A test/dev database with TimescaleDB
./scripts/test-db.sh          # podman; prints TEST_DATABASE_URL

# 2. Migrations (refinery, the same path the server uses)
cargo run -p migrations

# 3. The Rust API
cargo run

# 4. The Python sidecar (HTTP app) and its worker
cd sidecar
uv run uvicorn app.main:app --port 8000
uv run python -m app.worker

# 5. Everything at once, with telemetry
podman compose -f Local.DockerCompose.yaml up
```

Environment: `TEST_DATABASE_URL` (tests), `DB_*` (server), `ANALYTICS_DB_URL`
(sidecar, RLS-scoped), `ANALYTICS_TRAIN_DB_URL` (training job, cross-tenant
role), `PASETO_ACTIVE_KID` / `PASETO_KEY_<KID>` (tokens), `APP_ORIGIN`
(CSRF anchor). `AUTH_SKIP_VERIFY` works only in debug builds.

## Tests

```bash
export TEST_DATABASE_URL='postgres://postgres:postgres@127.0.0.1:55432/postgres'
cargo test                    # 227 integration + unit tests
cd sidecar && uv run --extra test pytest -q   # 84 tests
make test-sidecar             # the CI environment (some tests skip without the DB)
```

Tests fail loudly without `TEST_DATABASE_URL` — a silently skipped integration
test is a test that has rotted.

## Documentation

| | |
|---|---|
| [`docs/plan/DECISIONS.md`](docs/plan/DECISIONS.md) | D1–D84, every decided question with its reasoning |
| [`docs/adr/`](docs/adr/) | Architecture decisions (ledger, transfers, runway, sessions) |
| [`docs/plan/`](docs/plan/) | Phases 0–9 and their status |
| [`GLOSSARY.md`](GLOSSARY.md) | The domain's language: Wallet, Purchase, Allocation, Reconciled… |
| [`docs/MONEY.md`](docs/MONEY.md) | The money rules |

## Status

Phases 0–3 complete. Open work is tracked as issues: phase 4 (anomalies and
duplicates), phase 5 (hardening), phase 6 remainder (back-testing), and phase 9
(split purchases, reconciliation, import, envelope budgeting, embeddings,
category groups, tags) — all specced and labelled `ready-for-agent`.
