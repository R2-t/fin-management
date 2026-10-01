# Analytics Investigation

**Status:** Investigation / design only. No code changed.
**Date:** 2025 (claims about TimescaleDB PG18 support and Community/TSL licensing verified against
Tiger Data release notes and edition docs).
**Scope:** PostgreSQL + TimescaleDB analytics platform for `financial-management`
**Features covered:** (1) cash-flow / spending forecasting, (2) auto-categorization & merchant cleaning,
(3) anomaly / duplicate / unintended-recurring detection, (4) historical trends.

This document is grounded in the current repo state:

- `ARCHITECTURE.md` (monolith: Axum → application services → repositories → Postgres).
- `migrations/migrations/U1..U5__*.sql`.
- `src/model/`, `src/repository/`, `src/application/`.
- `Local.DockerCompose.yaml`.
- `prompts/multi-currency.md`.

---

## 1. Executive summary

The app today is an OLTP CRUD service for wallets, purchases, items, categories and an audit
trail. It has **no analytics substrate and, more importantly, several missing domain inputs**
that analytics needs: there is no wallet balance/ledger, no budgets, no income/payday schedule,
no recurring/subscription rules, no merchant entity, no user profile (timezone/base currency),
and no feedback tables. `purchase` is a mutable, soft-deleted, versioned table — a poor fit for a
TimescaleDB hypertable directly.

Recommendation:

1. **Do not hypertable the OLTP `purchase` table.** Keep it as-is and add an append-only
   **`purchase_fact` hypertable** (a derived analytics projection) populated from the OLTP write
   path. This side-steps all PK/partition-key and foreign-key constraints and keeps money math
   out of the hot path.
2. **Add the missing domain tables first** (profile, merchant, ledger/balance, budget,
   income schedule, recurring rules, feedback). Most of features 1–3 are impossible without them.
3. **Most aggregation belongs in SQL**; **orchestration and rules belong in Rust**; **ML, NLP and
   fuzzy/statistical modeling belong in one small Python sidecar** (scikit-learn, `rapidfuzz`).
   Do not build a general analytics microservice per feature.
4. **Start with live `time_bucket`/window SQL queries and a daily rollup**, and only add
   continuous aggregates once volume justifies the operational cost. For a personal-finance row
   count, "Postgres + one rollup table" is often enough.
5. Honest caveat: the current database volume is small. TimescaleDB buys compression, retention
   and `time_bucket` convenience; it does **not** fix the missing domain data. If the product is
   single-tenant/low-volume, plain Postgres matviews are a legitimate shortcut.

---

## 2. Current state (facts that constrain the design)

### 2.1 Tables and view

| Object | Key facts relevant to analytics |
|---|---|
| `wallet` | `id UUID PK`, `user_id`, `name`, `currency CHAR(3) DEFAULT 'USD'`, `deleted_at`. **No balance.** |
| `category` | `id INTEGER PK (identity)`, `name`, `user_id NULL`, `is_system`. Shared system + per-user categories. |
| `purchase` | `id UUID PK`, `user_id`, `wallet_id FK`, `category_id FK`, free-text `store`, `description`, `notes`, `version`, `currency CHAR(3)`, `exchange_rate DOUBLE PRECISION`, `rate_source`, `rate_at`, `purchase_date TIMESTAMPTZ NOT NULL`, `deleted_at`. Mutable + soft-deleted. |
| `item` | composite PK `(purchase_id, item_sequence)`, FK `purchase_id → purchase(id)`, `unit_price BIGINT` (minor units, i64), `quantity INTEGER > 0`, `item_type`. |
| `purchase_history` | `id`, `purchase_id FK`, `version`, `changes JSONB`, `changed_at`. Append-only audit. |
| `exchange_rate_cache` | `UNLOGGED`, PK `(from_currency, to_currency, rate_hour)`, `rate DOUBLE PRECISION`, `stale`. |
| `purchase_items` (view) | Joins purchase/item/category/wallet; computes `sum(quantity * unit_price)` as `total` and `* COALESCE(exchange_rate,1)` as `total_wallet`. Totals are **derived, never stored** (per `prompts/multi-currency.md`). |

### 2.2 Code shape

- `src/repository/*` uses raw `tokio-postgres` with a `PgPool` (bb8). No ORM. Money is `i64`
  minor units for items; FX rate is `f64`.
- `src/application/inbound/repositories.rs` defines repo traits; `PurchaseService` is generic over
  `P: PurchaseRepository, I: ItemRepository, R: RateCacheRepository, W: WalletRepository, F: ExchangeRateProvider`.
- Writes are transactional at the repository boundary (purchase + items; version bump + history).
- Auth middleware yields `AuthPrincipal { user_id }` (dev bypass `StaticUserVerifier`).
  Analytics endpoints will get `user_id` the same way and must filter on it.
- No user table exists at all — only opaque `user_id UUID` values.

### 2.3 Infrastructure

- `Local.DockerCompose.yaml` runs **plain `postgres:18.3-alpine`**, not TimescaleDB.
- OTLP collector, Jaeger, Loki, Prometheus are already wired; the Python sidecar can plug into the
  same collector.
- Migrations are `refinery` with embedded SQL, sequential `U<n>__name.sql`.

### 2.4 Gaps that block each feature

| Needed input | Exists? | Blocks |
|---|---|---|
| Wallet balance / ledger | ❌ | cash runway, budget burn, recurring impact |
| Wallet currency settable via API | ❌ — `create/update wallet` accept `name` only, so every API-created wallet is `'USD'` | meaningful multi-currency `base_currency` (see `docs/reports/api-review.md` P1-6); blocks non-USD reporting even though the schema supports it |
| Budgets | ❌ | overspend forecasting |
| Payday / income schedule | ❌ | run-out-of-cash forecast |
| Recurring/subscription rules | ❌ | unintended recurring charge detection |
| Merchant entity | ❌ (only `purchase.store` free text) | cleaning, classification, duplicate/recurrence keys |
| User profile (timezone, base currency) | ❌ | day bucketing, pay periods, reporting currency |
| Classification + feedback | ❌ | ML training loop |
| Analytics job/outbox | ❌ | async Python boundary, backfills |
| Fact/rollup tables | ❌ | trends performance, forecasting features |

---

## 3. TimescaleDB migration and schema strategy

### 3.1 Image / extension migration

Replace the Postgres service image:

```yaml
# Local.DockerCompose.yaml (proposal)
postgresdb:
  image: timescale/timescaledb:2.23.0-pg18   # or timescale/timescaledb-ha:pg18.x-ts2.23.0
  # TimescaleDB requires shared_preload_libraries=timescaledb (the image sets this).
  # Data volume is compatible with the existing postgres:18.3 data directory.
```

Notes / verified facts (checked against the vendor release notes/docs):

- **TimescaleDB 2.23.0 (released 2025-10-29) is the first release with full PostgreSQL 18
  support** and supports PG 15/16/17/18. The current `postgres:18.3` is therefore compatible,
  but pin the Timescale image tag (`timescale/timescaledb:2.23.0-pg18` or the `-ha` variant) and
  do not float it.
- **Continuous aggregates, retention policies, and the columnstore/compression are Community (TSL)
  features, not Apache-2.** The default `timescale/timescaledb` tag enables them; a
  `timescaledb-apache`/`timescaledb.license=apache` build does **not**. Confirm licensing with
  `SHOW timescaledb.license;` (expect `timescale`) before relying on CAGGs.
- Both the stock PG18 and Timescale PG18 images keep the same `PG_VERSION`/catalog layout, so the
  existing `postgresdb-data` volume should open unmodified — but verify on a copy before deploy,
  and remember TimescaleDB requires the extension in `shared_preload_libraries` (the image sets it)
  which means a **restart** of the database during the image swap.
- Migration is additive: `CREATE EXTENSION IF NOT EXISTS timescaledb;` in a new `U6` migration.
  Existing tables are untouched.

### 3.2 Why **not** to convert `purchase` into a hypertable

Verified TimescaleDB constraints:

- Every unique/primary index on a hypertable **must include every partitioning column**.
  Partitioning `purchase` by `purchase_date` forces `PRIMARY KEY (id, purchase_date)`, so
  `purchase(id)` is no longer unique.
- Foreign keys **from** a regular table **to** a hypertable are supported only when they point at
  a unique index that includes the partition key. Therefore `item.purchase_id → purchase(id)` and
  `purchase_history.purchase_id → purchase(id)` would break/require denormalizing `purchase_date`
  into both child tables and composite FKs.
- Hypertables favor append-only data. `purchase` is updated (version bumps, soft delete) and
  compressed chunks cannot be updated. This fights the model.
- `create_hypertable(..., migrate_data => TRUE)` on a table with FKs risks deadlocks and long locks.

### 3.3 Recommended: append-only `purchase_fact` hypertable

Keep OLTP tables unchanged. Add a **derived, append-only** projection:

```sql
CREATE TABLE purchase_fact (
    occurred_at        TIMESTAMPTZ NOT NULL,          -- = purchase.purchase_date (event time)
    purchase_id        UUID        NOT NULL,
    version            INT         NOT NULL,
    user_id            UUID        NOT NULL,
    wallet_id          UUID        NOT NULL,
    category_id        INT         NOT NULL,
    merchant_id        UUID,
    source             TEXT        NOT NULL,          -- 'purchase' | 'income' | 'transfer' | 'recurring'
    deleted            BOOLEAN     NOT NULL DEFAULT FALSE,
    currency           CHAR(3)     NOT NULL,          -- source currency
    amount_minor       BIGINT      NOT NULL,          -- sum(quantity*unit_price), source currency, minor units
    exchange_rate_scaled BIGINT,                      -- rate * 1e9 (avoid f64), NULL when same currency
    base_currency      CHAR(3)     NOT NULL,          -- reporting/wallet currency
    amount_base_minor  BIGINT      NOT NULL,          -- rounded converted amount, minor units
    PRIMARY KEY (occurred_at, purchase_id, version)   -- includes partition key
);

SELECT create_hypertable('purchase_fact', by_range('occurred_at', INTERVAL '1 month'),
                         if_not_exists => TRUE);
CREATE INDEX ON purchase_fact (user_id, occurred_at DESC);
CREATE INDEX ON purchase_fact (user_id, category_id, occurred_at DESC);
CREATE INDEX ON purchase_fact (user_id, merchant_id, occurred_at DESC);
```

Properties:

- **Append-only**: corrections insert a new `version`; readers take the latest version per
  `purchase_id` (or the fact builder deletes+re-inserts the row and a refresh job runs).
- `amount_base_minor` is materialized **only in the analytics projection**, not in OLTP. This
  respects the `prompts/multi-currency.md` rule that OLTP totals stay derived; the projection is
  explicitly a reporting artifact.
- Regular→hypertable FKs are not needed because `purchase_fact` references nothing.
- Optionally add `item_fact` (same pattern, keyed by `(occurred_at, purchase_id, item_sequence)`)
  for item-level price-anomaly detection. Not required initially.

**Population options** (pick one, in order of preference):

1. **Same-transaction dual-write in the repository** (mirrors the existing history-write pattern).
   The fact row is written in the same `tran` as the purchase/item mutation. Strong consistency,
   no trigger magic, easy to test.
2. **DB trigger** on `purchase`/`item` that upserts the fact row. Guarantees consistency even for
   manual SQL, but hides logic from the codebase and complicates migrations.
3. **Outbox + dispatcher**: `analytics_outbox` written transactionally, a background task projects
   into `purchase_fact`. Best if the Python side also consumes events later; more moving parts.

For this codebase, option 1 fits the established pattern (`insert_history` inside a transaction).

### 3.4 Continuous aggregates

Once daily/monthly rollups are worth it (see §8 for the "not yet" caveat):

```sql
CREATE MATERIALIZED VIEW purchase_daily
WITH (timescaledb.continuous) AS
SELECT time_bucket('1 day', occurred_at) AS bucket,
       user_id, wallet_id, category_id, base_currency,
       sum(amount_base_minor) AS amount_minor,
       count(*)               AS txn_count
FROM purchase_fact
WHERE deleted = FALSE
GROUP BY bucket, user_id, wallet_id, category_id, base_currency;

SELECT add_continuous_aggregate_policy('purchase_daily',
    start_offset => INTERVAL '3 months',
    end_offset   => INTERVAL '1 day',
    schedule_interval => INTERVAL '1 hour');
```

Constraints to design around:

- CAGGs cannot use window functions (`lag`, `rank`), `DISTINCT`, or arbitrary joins. Compute MoM
  and rolling averages **on top of** the CAGG with ordinary window functions.
- CAGGs do not automatically retract data when a fact row is deleted/mutated. Corrections to
  recent periods must call `refresh_continuous_aggregate('purchase_daily', from, to)`.
- Custom date ranges are served by re-aggregating daily CAGG rows (arbitrary `WHERE bucket >= …`),
  optionally unioning today's live `purchase_fact` for freshness.
- **Columnstore (compression) policy** for `purchase_fact` (also TSL) is worth adding once chunks
  age, e.g. `ALTER TABLE purchase_fact SET (timescaledb.enable_columnstore = true, timescaledb.segmentby = 'user_id, category_id', timescaledb.orderby = 'occurred_at DESC');` plus
  `SELECT add_columnstore_policy('purchase_fact', AFTER => INTERVAL '30 days');` (the older
  `timescaledb.compress*` aliases and `add_compression_policy` still work but are deprecated as of
  2.18). Compressed chunks are read-only, which is fine because facts are append-only by design.
  `retention` is optional here — personal finance history is usually kept indefinitely; if raw
  facts are ever dropped, keep the CAGG/rollups first (`add_retention_policy` only after aggregates
  exist).

### 3.5 Money & multi-currency conventions

- Keep **`BIGINT` minor units** end-to-end. Never introduce `FLOAT`/`DOUBLE PRECISION` for money.
- Replace the lossy `exchange_rate DOUBLE PRECISION` with either `NUMERIC(20,10)` or a scaled
  `BIGINT` (`rate_micros`/`rate_nanos`). This is a Rust change too: `Purchase.exchange_rate` is
  `Option<f64>` today. If changing that is disruptive, at minimum copy the rate as scaled bigint
  into `purchase_fact`.
- Define **one rounding rule** and apply it once per purchase:
  `amount_base_minor = round_half_up(amount_minor * rate)`. Do not round each line item and then
  sum — that drifts by minor units and violates the multi-currency consistency requirement
  ("sum of converted line items must exactly match the wallet deduction").
- Each fact row carries `base_currency`; all cross-currency aggregates group by it. Never sum
  mixed currencies.
- Historical rates are already denormalized onto `purchase` at creation (`rate_source`, `rate_at`);
  the fact table inherits them, so historical reports are stable and reproducible.

### 3.6 Missing-data schema (proposed, `U6`+)

```sql
-- User-level defaults missing entirely today.
CREATE TABLE user_profile (
    user_id       UUID PRIMARY KEY,
    timezone      TEXT NOT NULL DEFAULT 'UTC',
    base_currency CHAR(3) NOT NULL DEFAULT 'USD',
    locale        TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Canonical merchant + aliases (free-text store is not enough).
CREATE TABLE merchant (
    id            UUID PRIMARY KEY,
    user_id       UUID NOT NULL,
    canonical_name TEXT NOT NULL,
    default_category_id INT REFERENCES category(id),
    is_subscription BOOLEAN NOT NULL DEFAULT FALSE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, canonical_name)
);
CREATE TABLE merchant_alias (
    merchant_id UUID NOT NULL REFERENCES merchant(id) ON DELETE CASCADE,
    raw_pattern TEXT NOT NULL,          -- matched against cleaned purchase.store
    match_kind  TEXT NOT NULL,          -- 'exact' | 'contains' | 'regex' | 'learned'
    PRIMARY KEY (merchant_id, raw_pattern)
);
ALTER TABLE purchase ADD COLUMN merchant_id UUID REFERENCES merchant(id);

-- Balance / ledger (wallet has no balance today).
CREATE TABLE ledger_entry (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL,
    wallet_id    UUID NOT NULL REFERENCES wallet(id),
    occurred_at  TIMESTAMPTZ NOT NULL,
    amount_minor BIGINT NOT NULL,       -- signed, minor units
    currency     CHAR(3) NOT NULL,
    kind         TEXT NOT NULL,         -- 'purchase' | 'income' | 'transfer' | 'adjustment'
    purchase_id  UUID,                  -- nullable back-reference
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Budgets (per category, or total when category_id IS NULL).
CREATE TABLE budget (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL,
    category_id  INT REFERENCES category(id),   -- NULL = overall budget
    period       TEXT NOT NULL,                 -- 'weekly' | 'monthly' | 'custom'
    amount_minor BIGINT NOT NULL,
    currency     CHAR(3) NOT NULL,
    starts_on    DATE NOT NULL,
    ends_on      DATE,                          -- for custom
    rollover     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Income / payday schedule.
CREATE TABLE income_schedule (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL,
    wallet_id    UUID REFERENCES wallet(id),
    name         TEXT NOT NULL,
    amount_minor BIGINT NOT NULL,
    currency     CHAR(3) NOT NULL,
    cadence      TEXT NOT NULL,          -- 'weekly' | 'biweekly' | 'semimonthly' | 'monthly'
    next_date    DATE NOT NULL,
    active       BOOLEAN NOT NULL DEFAULT TRUE
);

-- Declared subscriptions/recurring commitments.
CREATE TABLE recurring_rule (
    id              UUID PRIMARY KEY,
    user_id         UUID NOT NULL,
    merchant_id     UUID REFERENCES merchant(id),
    category_id     INT REFERENCES category(id),
    amount_minor    BIGINT NOT NULL,
    currency        CHAR(3) NOT NULL,
    cadence         TEXT NOT NULL,
    next_expected   DATE NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active', -- 'active' | 'cancelled' | 'ended'
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ML feedback + predictions.
CREATE TABLE purchase_classification (
    purchase_id     UUID PRIMARY KEY REFERENCES purchase(id),
    predicted_category_id INT REFERENCES category(id),
    confidence      REAL,
    model_version   TEXT,
    source          TEXT NOT NULL,       -- 'model' | 'rule' | 'user' | 'pending'
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE classification_feedback (
    id              UUID PRIMARY KEY,
    purchase_id     UUID NOT NULL REFERENCES purchase(id),
    predicted_category_id INT,
    corrected_category_id INT NOT NULL,
    model_version   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Findings surfaced to the user.
CREATE TABLE anomaly (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL,
    kind         TEXT NOT NULL,          -- 'amount_spike' | 'new_merchant' | 'duplicate_candidate' | 'unintended_recurring'
    severity     REAL NOT NULL,
    purchase_ids UUID[] NOT NULL,
    reason       JSONB NOT NULL,
    status       TEXT NOT NULL DEFAULT 'open', -- 'open' | 'dismissed' | 'confirmed'
    detected_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Generated forecasts, versioned so they can be evaluated/compared.
CREATE TABLE forecast (
    id            UUID PRIMARY KEY,
    user_id       UUID NOT NULL,
    generated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    horizon_days  INT NOT NULL,
    kind          TEXT NOT NULL,         -- 'budget_overspend' | 'cash_runway'
    target        JSONB NOT NULL,        -- {date, amount_minor, probability, ...}
    model_version TEXT NOT NULL
);

-- Async job queue + training runs (avoid Kafka for this scale).
CREATE TABLE analytics_job (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID,
    kind         TEXT NOT NULL,          -- 'classify' | 'anomaly_scan' | 'recurrence_scan' | 'retrain'
    payload      JSONB NOT NULL DEFAULT '{}',
    status       TEXT NOT NULL DEFAULT 'queued', -- queued|running|done|failed
    attempts     INT NOT NULL DEFAULT 0,
    run_after    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE model_registry (
    model_version TEXT PRIMARY KEY,
    trained_at    TIMESTAMPTZ NOT NULL,
    metrics       JSONB NOT NULL,
    active        BOOLEAN NOT NULL DEFAULT FALSE
);
```

### 3.7 Migration path from current tables

1. `U6__timescaledb.sql`: `CREATE EXTENSION timescaledb;` and `CREATE TABLE purchase_fact` +
   `create_hypertable`. (Extension creation should be idempotent.)
2. `U7__analytics_domain.sql`: profile, merchant, ledger, budget, income, recurring, feedback,
   anomaly, forecast, job/registry tables.
3. `U8__fact_backfill.sql` (data migration / one-off job): build `purchase_fact` from existing
   `purchase JOIN item`, resolving `user_profile.base_currency` (fallback wallet currency) and
   copying the denormalized rate. Run in batches; do **not** block deploys.
4. `U9__aggregates.sql` (optional, later): `purchase_daily`/`purchase_monthly` + policies.

Every migration stays additive; OLTP repos keep working unchanged. The only OLTP change is the
nullable `purchase.merchant_id` and the (optional) rate type change, both of which need Rust
touch-ups.

---

## 4. Feature 1 — Cash-flow & spending forecasting

### 4.1 What is being predicted

- **Budget overspend**: "given spend so far this period, will `budget.amount_minor` be exceeded
  before `ends_on`, and by how much / with what probability?"
- **Run out of cash**: "given current wallet balance, scheduled income, committed recurring
  charges and discretionary burn, on what date does `balance < 0` before `next_payday`?"

### 4.2 Data pipeline

- Historical spend comes from `purchase_fact` (or live `purchase_items`).
- Commitments come from `recurring_rule`.
- Income from `income_schedule`.
- Balance from `ledger_entry` (sum by wallet) or a cached `wallet_balance` rollup.

### 4.3 Methods, by tier

| Tier | Method | Where |
|---|---|---|
| Baseline | Trailing burn rate: `spend_last_N_days / N`, projected linearly; compare to budget remaining | SQL + Rust orchestration |
| Better | Robust daily seasonal profile: median spend per weekday, scaled to remaining days; excludes one-off anomalies | SQL window functions |
| Cash runway | Deterministic daily simulation: `balance +Σincome − Σrecurring − burn`; first negative date | Rust (integer math) over SQL aggregates |
| Probabilistic | Residual bootstrap / quantile regression / Prophet-style decomposition → overspend probability and P10/P50/P90 runway | Python (statsmodels/Prophet or scikit-learn quantile regressors) |

Rules of thumb:

- Keep the deterministic runway simulation in **Rust** using integer minor units and SQL
  aggregates — it must be exact and auditable.
- Only add Python when the product needs distributions/probabilities. Persist results in
  `forecast` with `model_version` so they can be back-tested.
- Cold start: with < ~1 month of history, fall back to the budget's straight-line burn and the
  declared recurring rules. Do not train ML on no data.

### 4.4 Output contract (illustrative)

```json
{
  "kind": "cash_runway",
  "base_currency": "USD",
  "current_balance_minor": 214300,
  "next_payday": "2025-10-15",
  "projected_zero_date": "2025-10-12",
  "shortfall_minor": 18200,
  "confidence": 0.72,
  "assumptions": { "burn_source": "trailing_30d", "includes_recurring": true },
  "model_version": "runway-v1"
}
```

---

## 5. Feature 2 — Auto-categorization & merchant cleaning (Python microservice)

### 5.1 Why Python

Merchant normalization and category prediction are NLP/ML problems: TF-IDF, fuzzy matching,
clustering, linear/SVM classifiers. Rust has no comparable mature ecosystem; SQL has no business
doing string clustering. This is the one place a Python sidecar is clearly justified.

### 5.2 Two sub-problems

1. **Merchant cleaning / normalization**
   - Deterministic transforms: lowercase, strip card-processor prefixes/suffixes
     (`SQ *`, `PAYPAL *`, `AMZN Mktp`, `TST*`, trailing store numbers, phone numbers, dates).
   - Fuzzy match against `merchant_alias` with `rapidfuzz` (`WRatio`/`token_set_ratio`).
   - Clustering unknown strings with char n-gram TF-IDF + DBSCAN/agglomerative to propose new
     canonical merchants; a human/model confirms.
   - Output: canonical `merchant_id` (or a proposed canonical name) + confidence.
2. **Category classification**
   - Features: cleaned merchant name, `store`/`description` tokens, item names, `item_type`, amount
     bucket, weekday. TF-IDF (char + word n-grams) → `LinearSVC`/`LogisticRegression`
     (calibrated) or `SGDClassifier` for incremental learning.
   - Cold start: keyword/rule priors and the merchant's `default_category_id`.
   - Output: `category_id` + confidence; below threshold → keep `Uncategorized` and ask the user.

### 5.3 Service boundary (Rust ↔ Python)

- Python service: FastAPI + scikit-learn + `rapidfuzz`, deployed as a sibling container on the
  same compose network; no public port. Ships OTLP traces to the existing collector.
- Rust adds an outbound trait (mirroring `ExchangeRateProvider`):

  ```rust
  // src/application/outbound/analytics.rs (proposal)
  pub trait SpendAnalytics: Send + Sync {
      async fn classify(&self, req: ClassifyRequest) -> Result<ClassifyResult, AnalyticsError>;
      async fn normalize_merchant(&self, raw: &str) -> Result<MerchantGuess, AnalyticsError>;
      async fn score_anomalies(&self, req: AnomalyRequest) -> Result<Vec<AnomalyScore>, AnalyticsError>;
  }
  ```

  `HttpAnalyticsClient` uses `reqwest` with a hard timeout (~150 ms classify), one retry, and a
  circuit breaker. On failure: fall back to rules/`Uncategorized` and enqueue an `analytics_job`.
- Contracts: versioned JSON, explicit `model_version` in every response, batch endpoints to avoid
  chatty calls. Propagate W3C `traceparent`.
- **Data access**: give Python a **read-only DB role**; it writes only to analytics-owned tables
  (`purchase_classification`, `anomaly`, `model_registry`) or returns results to Rust. It must
  never mutate `purchase`/item/ledger directly, and must always filter by `user_id`.
- Sync vs async:
  - **Async (recommended):** purchase create assigns the system `Uncategorized` category (or the
    merchant default), enqueues classify; Python/Rust updates `purchase.category_id` +
    `purchase_classification` later, writing feedback when the user corrects it.
  - **Sync (optional):** only when `category_id` is omitted; must gracefully degrade.
  - Note: `purchase.category_id` is currently `NOT NULL`, so "classify later" means either a
    dedicated system `Uncategorized` category or making the column nullable — a small but explicit
    schema/API decision.

### 5.4 Training & feedback loop

1. **Capture**: every user category edit that differs from the prediction inserts
   `classification_feedback` (predicted vs corrected). This is the highest-value label source.
2. **Aggregate**: training job pulls feedback + historically stable purchases (e.g., unchanged for
   > 30 days) as weak labels.
3. **Train**: scheduled (weekly) offline job, time-based train/validation split (never random —
   temporal leakage). Evaluate accuracy + macro-F1 + per-class recall.
4. **Register**: write `model_registry(model_version, metrics, active=false)`.
5. **Evaluate**: compare against the active model on a held-out *recent* window.
6. **Promote**: set `active=true` only if metrics clear a threshold; keep rollback by flipping the
   flag. Optionally shadow-serve the candidate first.
7. **Per-user vs global**: start with a global model + per-user merchant aliases/defaults; add
   per-user personalization only when a user has enough labels.

---

## 6. Feature 3 — Anomaly, duplicate & unintended-recurring detection

### 6.1 Exact / near-duplicate detection

- **Exact (SQL)**: self-join on `purchase_fact` by `(user_id, amount_base_minor, merchant_id)`
  within a short window (e.g., ±48 h) and same currency → `duplicate_candidate`.
- **Near (Python)**: fuzzy merchant match + amount tolerance (e.g., ±2%) + time window;
  detect split transactions (same merchant, adjacent timestamps, amounts summing to one).
- Persist in `anomaly(kind='duplicate_candidate')` with the involved `purchase_ids`; let the user
  dismiss/confirm, and feed dismissals back to reduce false positives.

### 6.2 Amount / behavior anomalies

- **Robust statistics (recommended first)**: per `(user, merchant/category)` median + MAD or IQR;
  flag `|x − median| > k·MAD`. Sparse personal data makes robust stats outperform deep models.
- **Multivariate (Python)**: `IsolationForest`/`LocalOutlierFactor` over
  `[amount, weekday, hour, merchant rarity, days_since_last]` for batch scoring.
- **New-merchant / first-seen** and **duplicate-day spikes** are simple SQL flags.
- Output `severity`, human-readable `reason` JSONB, and status. Never auto-delete; always surface.

### 6.3 Unintended recurring charges

Logic:

1. Group `purchase_fact` by `merchant_id` (or cleaned merchant), order by `occurred_at`.
2. Compute inter-arrival gaps (`lag` window function) and amount variance.
3. A merchant is "recurring-like" if gaps cluster around a cadence (weekly/±2 d, monthly/±4 d) and
   amounts are stable (low coefficient of variation) over ≥ 3 occurrences.
4. Compare detected recurrence against declared `recurring_rule`:
   - detected but **not declared** → "possible subscription you haven't tracked";
   - declared but **amount/date changed** → "price change / failed payment";
   - declared `cancelled` but still occurring → "you may still be paying for this".
5. Persist as `anomaly(kind='unintended_recurring')`.

Split: SQL does grouping/intervals; Python handles cadence fitting/periodicity if needed. For most
cases, SQL window functions are sufficient.

---

## 7. Feature 4 — Historical trends

### 7.1 Requirements → query shape

| Requirement | Implementation |
|---|---|
| Category breakdown over custom range | `SELECT category_id, sum(amount_base_minor) FROM purchase_fact WHERE user_id=$1 AND occurred_at >= $2 AND occurred_at < $3 GROUP BY 1` |
| Month-over-month growth | Aggregate monthly, `lag() OVER (ORDER BY month)` → `(cur-prev)/prev` |
| Rolling averages (7/30/90 d) | `avg(...) OVER (ORDER BY bucket ROWS BETWEEN N PRECEDING AND CURRENT ROW)` over daily rollup/CAGG |
| Custom ranges | Re-aggregate the daily rollup (`bucket >= start AND bucket < end`), union live fact for the partial current day |
| Multi-currency | Always filter/group by `base_currency`; convert at fact-build time, not query time |

### 7.2 Performance path

1. **Now**: `purchase_fact` with `(user_id, occurred_at)` indexes + live `time_bucket` queries.
2. **Later**: `purchase_daily` CAGG; compute MoM/rolling on top with window functions.
3. **Freshness**: union today's live rows with the CAGG for the current bucket, or refresh the CAGG
   hourly with `end_offset => 0`.
4. **Money type in the API**: `sum(BIGINT)` returns `NUMERIC`; serialize as a **string** or `i64`
   in JSON, never `f64`, to avoid precision loss.

### 7.3 Timezone

"Daily"/"monthly" and pay periods must use `user_profile.timezone`, not UTC. `time_bucket` has a
`timezone` argument; use it once profile support lands. This is a correctness issue, not cosmetic.

---

## 8. Rust ↔ Python boundary summary

```
┌──────────────┐   HTTP/JSON (internal net, timeout + circuit breaker)   ┌──────────────────┐
│ Rust (Axum)  │ ───────────────────────────────────────────────────────▶ │ Python sidecar   │
│              │   classify / normalize_merchant / score_anomalies /     │ FastAPI +        │
│ system of    │   train (trigger)                                        │ scikit-learn +   │
│ record,      │ ◀─────────────────────────────────────────────────────── │ rapidfuzz        │
│ orchestrator │   category_id + confidence + model_version               │ statsmodels      │
└──────┬───────┘                                                          └────────┬─────────┘
       │ writes OLTP + purchase_fact (same tx)                                   │ read-only DB role
       │                                                                          │ writes analytics tables
       ▼                                                                          ▼
┌──────────────────────────────── PostgreSQL + TimescaleDB ─────────────────────────────────┐
│ OLTP: wallet, category, purchase, item, purchase_history, exchange_rate_cache            │
│ Domain: user_profile, merchant(_alias), ledger_entry, budget, income_schedule,           │
│         recurring_rule, purchase_classification, classification_feedback                 │
│ Analytics: purchase_fact (hypertable), purchase_daily (CAGG), anomaly, forecast,         │
│            analytics_job, model_registry                                                  │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

Ownership rules:

- Rust owns all writes to OLTP and `purchase_fact`; owns auth, validation, integer money math, and
  business rules/fallbacks.
- Python owns model artifacts, inference, batch scoring, and training; reads via read-only role.
- Only Rust writes user-visible `purchase.category_id`; Python's suggestions are staged.
- Results are versioned (`model_version`) and explainable (`reason`/`assumptions`).

---

## 9. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| TimescaleDB CAGGs/compression are TSL, not Apache-2 | Licensing / self-host constraints | Confirm edition (`SHOW timescaledb.license`), pin image, avoid `timescaledb-apache` if CAGGs needed |
| `postgres:18.3` vs Timescale version drift | Extension won't load | Pin `timescaledb:2.23.0-pg18`; verify at deploy |
| Hypertabling `purchase` breaks PK/FK | Failed migration / corrupted model | Use append-only `purchase_fact`, never convert `purchase` |
| Volume is tiny | Timescale ops cost > benefit | Start with plain tables + live `time_bucket`; add CAGGs only if needed |
| Wallet currency cannot be set through the API (defaults to USD) | Multi-currency facts have a single `base_currency`; reports are meaningless for non-USD holders | Fix `Create/UpdateWallet` to accept/validate ISO-4217 `currency` before building multi-currency analytics (api-review P1-6) |
| No balance/ledger/income data | Runway forecast is meaningless | Build domain tables first; block feature 1 on them |
| `f64` exchange rates | Minor-unit drift, non-reproducible reports | Scaled bigint rates; one rounding rule; audit sums |
| Mixed-currency aggregation | Wrong totals | Materialize `amount_base_minor` + `base_currency`; always group by currency |
| Timezone-unaware buckets | Off-by-one-day trends, wrong pay periods | Store `user_profile.timezone`; use `time_bucket` tz arg |
| ML cold start / class imbalance | Bad predictions, user distrust | Rules + merchant defaults first; confidence thresholds; `Uncategorized` |
| Feedback bias | Model overfits corrections | Sample implicit labels carefully; temporal splits; monitor macro-F1 |
| Privacy / data isolation | Financial text leakage across users | Read-only scoped DB role, `user_id` on every query, no raw PII in logs/traces |
| Duplicate false positives | Alert fatigue | Conservative thresholds; user dismiss feedback; never auto-delete |
| Corrections vs CAGG staleness | Stale trends after edits | Refresh recent buckets; or serve live for recent windows |
| Async classification changes category after creation | API/UX surprise | Explicit `classification_status`; show "suggested" state |
| Python sidecar unavailability | Create path degraded | Hard timeouts, circuit breaker, rule fallback, job retry |

---

## 10. What is NOT worth doing in SQL vs Rust vs Python

| Concern | SQL | Rust | Python |
|---|---|---|---|
| Set-based aggregation, grouping, joins | ✅ primary | | |
| `time_bucket`, window functions, CAGGs | ✅ primary | | |
| Exact duplicate self-joins | ✅ | | |
| Money arithmetic, rounding, invariants | ⚠️ only as `BIGINT` | ✅ authoritative | |
| API, auth, validation, orchestration | | ✅ | |
| Transactional writes + fact projection | | ✅ | |
| Rule-based category fallbacks | | ✅ | |
| Deterministic cash-runway simulation | aggregates only | ✅ | |
| ML classification / NLP / merchant clustering | ❌ | ❌ | ✅ |
| Fuzzy string matching at scale | ❌ | ⚠️ possible, poor fit | ✅ |
| Statistical anomaly models (IsolationForest/LOF) | ❌ | ❌ | ✅ |
| Probabilistic forecasting (bootstrap/Prophet) | ❌ | ❌ | ✅ |
| Model training / lifecycle / evaluation | ❌ | ❌ | ✅ |
| Streaming/Kafka, real-time scoring infra | ❌ | ❌ | ❌ (not needed at this scale) |
| High-QPS inference serving | | | ❌ (Python is fine at personal-finance QPS) |
| Heavy JSON feature engineering | ⚠️ painful | | ✅ |

Guiding rule: **SQL for aggregation, Rust for control and money, Python for learning and fuzziness.**

---

## 11. Proposed work breakdown

Sizes: S ≈ ≤1 day, M ≈ 2–4 days, L ≈ 1–2 weeks. "Layer" = primary owner.

### Phase 0 — Foundations (blocks everything)

| ID | Item | Layer | Depends on | Size |
|---|---|---|---|---|
| A0.1 | Switch compose to `timescale/timescaledb:2.23.0-pg18`; add `U6` `CREATE EXTENSION`; verify `SHOW timescaledb.license` | Infra/SQL | — | S |
| A0.2 | Rounding & money/currency conventions doc (scaled rates, one round rule, reconciliation test) | Design | — | S |
| A0.3 | `user_profile` (timezone, base_currency, locale) + CRUD/endpoint + backfill default | SQL/Rust | A0.1 | S–M |
| A0.4 | `merchant` + `merchant_alias` + nullable `purchase.merchant_id`; seed from distinct `purchase.store` | SQL/Rust | A0.1 | M |
| A0.5 | `purchase_fact` hypertable + indexes; `U7` migration | SQL | A0.1, A0.2 | M |
| A0.6 | Fact projection in the write path (same tx as purchase/item mutations) | Rust | A0.5 | M |
| A0.7 | Backfill job for existing purchases (`U8` one-off, batched) | Rust/SQL | A0.6 | M |
| A0.8 | Reconciliation tests: OLTP totals == fact totals, per currency, rounding | Rust/Test | A0.6, A0.7 | M |
| A0.9 | Analytics repository module (typed queries over fact) | Rust | A0.6 | M |
| A0.10 | (Optional) `item_fact` hypertable for item-level analysis | SQL | A0.5 | M |

### Phase 1 — Historical trends (feature 4; fastest user value)

| ID | Item | Layer | Depends on | Size |
|---|---|---|---|---|
| A1.1 | `GET /analytics/trends` (range, category, wallet, currency filters) | Rust | A0.9 | M |
| A1.2 | Category breakdown + totals over custom range | Rust/SQL | A1.1 | S |
| A1.3 | Month-over-month growth (window `lag`) | Rust/SQL | A1.1 | S |
| A1.4 | Rolling averages (7/30/90 d) | Rust/SQL | A1.1 | S |
| A1.5 | Serialize money as string/i64 (never f64); contract tests | Rust | A1.1 | S |
| A1.6 | Optional `purchase_daily` CAGG + refresh policy | SQL/Infra | A0.5, A1.1 | M |
| A1.7 | Trend integration tests + seed dataset | Test | A1.1–A1.5 | M |

### Phase 2 — Budgets, paydays & forecasting (feature 1)

| ID | Item | Layer | Depends on | Size |
|---|---|---|---|---|
| A2.1 | `ledger_entry` (or wallet balance rollup) + write on purchase/income | SQL/Rust | A0.3 | L |
| A2.2 | `budget` table + CRUD endpoints | SQL/Rust | A0.3 | M |
| A2.3 | `income_schedule` table + CRUD; next-payday computation | SQL/Rust | A0.3 | M |
| A2.4 | `recurring_rule` table + CRUD | SQL/Rust | A0.4 | M |
| A2.5 | Budget burn-rate + overspend projection endpoint (SQL + Rust) | Rust/SQL | A2.2, A0.9 | M |
| A2.6 | Deterministic cash-runway simulation endpoint | Rust | A2.1, A2.3, A2.4 | M |
| A2.7 | `forecast` persistence + model versioning + back-test harness | SQL/Rust | A2.5, A2.6 | M |
| A2.8 | Seasonal weekday profile (robust median) | SQL/Rust | A2.5 | M |
| A2.9 | Python probabilistic forecasting (quantile/bootstrap/Prophet) | Python | A2.7, A3.1 | L |
| A2.10 | Forecast endpoints + tests | Rust | A2.5–A2.8 | M |

### Phase 3 — Python sidecar, merchant cleaning & categorization (feature 2)

| ID | Item | Layer | Depends on | Size |
|---|---|---|---|---|
| A3.1 | Python service scaffold: FastAPI, container, health, OTLP, read-only DB role | Python/Infra | A0.1 | M |
| A3.2 | Merchant normalization pipeline (rules + `rapidfuzz` + clustering proposals) | Python | A3.1, A0.4 | L |
| A3.3 | Category classifier baseline (TF-IDF + LinearSVC), offline eval harness | Python | A3.1 | L |
| A3.4 | Rust `SpendAnalytics` trait + `HttpAnalyticsClient` (timeout, retry, circuit breaker, fallback) | Rust | A3.1 | M |
| A3.5 | `purchase_classification` table + async classify job + `Uncategorized` category decision | SQL/Rust/Python | A3.4, A0.1 | L |
| A3.6 | `classification_feedback` capture on user category edits | SQL/Rust | A3.5 | M |
| A3.7 | Training pipeline + `model_registry` + scheduled retrain job + promotion gate | Python | A3.3, A3.6 | L |
| A3.8 | Shadow/A-B evaluation + accuracy dashboards | Python/Rust | A3.7 | M |
| A3.9 | Merchant cleaning write-back (canonical `merchant_id` on purchases) | Rust/Python | A3.2, A0.4 | M |

### Phase 4 — Anomaly, duplicates & unintended recurring (feature 3)

| ID | Item | Layer | Depends on | Size |
|---|---|---|---|---|
| A4.1 | Exact duplicate self-join → `anomaly(kind=duplicate_candidate)` | SQL/Rust | A0.9 | M |
| A4.2 | Near-duplicate / split-transaction detection | Python | A3.1, A4.1 | M |
| A4.3 | Robust amount anomalies (median/MAD per merchant/category) | SQL/Rust | A0.9 | M |
| A4.4 | Multivariate anomaly scoring (IsolationForest/LOF) batch job | Python | A3.1, A4.3 | M |
| A4.5 | Recurrence detection (interval clustering + amount stability) | SQL/Rust | A2.4, A0.9 | L |
| A4.6 | Declared-vs-detected recurrence comparison (undeclared/changed/cancelled) | Rust | A4.5 | M |
| A4.7 | Candidate review endpoints + dismiss/confirm feedback capture | Rust/SQL | A4.1, A4.3, A4.6 | M |
| A4.8 | Optional notifications (email/push) for high-severity findings | Rust | A4.7 | M |

### Phase 5 — Hardening

| ID | Item | Layer | Depends on | Size |
|---|---|---|---|---|
| A5.1 | Privacy/PII policy for analytics; scoped DB roles; redact logs/traces | Cross | A3.1 | M |
| A5.2 | Trace propagation Rust→Python; Python OTLP spans/metrics | Cross | A3.1 | S–M |
| A5.3 | Backfill + migration runbook; rollback plan | Ops | A0.7 | M |
| A5.4 | Performance tests on `purchase_fact`/CAGGs at projected volume | Test | A1.6 | M |
| A5.5 | Job runner/scheduler for `analytics_job` (retry, dead-letter) | Rust | A3.5 | M |

### Dependency spine (abbreviated)

```
A0.1 ─┬─ A0.3 ─┬─ A2.1 ─ A2.6 ─ A2.10
      │        ├─ A2.2 ─ A2.5 ─┘
      │        └─ A2.3 ─┘
      ├─ A0.5 ─ A0.6 ─ A0.7 ─ A0.8 ─ A0.9 ─┬─ A1.x (trends)
      │                                     ├─ A2.5
      │                                     ├─ A4.1/A4.3/A4.5
      │                                     └─ A3.5
      ├─ A0.4 ─┬─ A3.2
      │        └─ A2.4 ─ A4.5
      └─ A3.1 ─┬─ A3.2/A3.3 ─ A3.7
               ├─ A3.4 ─ A3.5 ─ A3.6 ─ A3.7
               └─ A4.2/A4.4
```

**Recommended sequencing:** Phase 0 → Phase 1 (quick, visible win) → Phase 2 (needs ledger/budget,
delivers the headline "don't run out of cash" feature) → Phase 3 (sidecar) → Phase 4 → Phase 5.
Trends first also produces the historical dataset the forecasting and anomaly models need.

### Requirement → design traceability

| Requested item | Where covered |
|---|---|
| (1) Cash-flow / spending forecasting (budget overspend, run out of cash before payday) | §4, §3.6 (`budget`, `income_schedule`, `recurring_rule`, `ledger_entry`, `forecast`) |
| (2) Auto-categorization & merchant cleaning (Python + scikit-learn) | §5, §3.6 (`merchant`/`merchant_alias`, `purchase_classification`, `classification_feedback`), §8 |
| (3) Anomaly, duplicate, unintended-recurring detection | §6, §3.6 (`anomaly`), §8 |
| (4) Historical trends (MoM, rolling averages, category breakdowns, custom ranges) | §7, §3.4 (CAGG), §3.3 (`purchase_fact`) |
| Hypertable on purchases? PK/partition-key constraints | §3.2 (answer: **no** hypertable on OLTP `purchase`), §3.3 (append-only `purchase_fact`) |
| Continuous aggregates | §3.4 |
| Migration path from current tables | §3.7 (`U6`–`U9`, backfill) |
| `BIGINT` money | §3.5 (minor units end-to-end; scaled integer FX rate) |
| Multi-currency (`prompts/multi-currency.md`) | §3.5, §4.3, §7.1, §2.4 (wallet-currency API gap) |
| Missing data: budgets, paydays, recurring rules, merchants | §2.4, §3.6 |
| Rust ↔ Python service boundary | §5.3, §8 |
| Training / feedback loop | §5.4 |
| Risks | §9 |
| What is *not* worth doing in SQL vs Rust vs Python | §10 |
| Small trackable work items with dependencies | §11 (Phases 0–5 + dependency spine) |
