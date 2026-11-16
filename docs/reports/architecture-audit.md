# Architecture Audit

**Scope:** `ARCHITECTURE.md` audited against the actual code in `src/**` (models, repositories,
services, commands, api, auth, config, otel) and the migrations `U1`–`U5` in
`migrations/migrations/`.
**Method:** read-only static review of the code; `ARCHITECTURE.md` was the only file edited. No
code was modified. `cargo check` was run to verify the doc's build claim (passes, no code
warnings — only an unrelated cargo `config` vs `config.toml` deprecation notice).
**Note on process:** a concurrent session applied corrections to the top half of the document
while this audit was running. This pass verified those edits against the code, applied the
remaining fixes, and produced this report. Everything below is the consolidated list of
discrepancies found between the previous `ARCHITECTURE.md` and the code, and how each was
resolved.

---

## 1. Corrections applied

### Implementation Status table (rewritten)
- Added rows for modules that exist in code but were absent from the doc:
  `src/application/outbound/exchange_rate.rs`, `src/application/purchase/commands.rs`,
  `src/application/purchase/diff.rs`, `src/application/purchase_history/service.rs`,
  `src/repository/rate_cache.rs`, `src/repository/purchase_history.rs`, `src/auth/`.
- `PurchaseService<P,I>` → `PurchaseService<P,I,R,W,F>` (rate cache, wallet, FX provider
  generics), and its method list corrected to create / get / list / update / add_items /
  update_item / remove_item / delete.
- `CategoryService` now also exposes `get`; `WalletService` now also exposes `get`.
- `src/repository/item.rs` note corrected: sequence comes from
  `MAX(item_sequence) + 1` per purchase (not an "auto-increment sequence"), and the price
  cast is NUMERIC→**BIGINT** mapped to `i64` (was "NUMERIC cast to f64").
- `RepositoryError` list corrected to include the `Serde` variant.
- Auth row: "Pending — `user_id` hardcoded as `Uuid::nil()`" was stale; auth is implemented
  (PASETO v4.local verifier + middleware + dev bypass). Pending items are now: real FX
  provider, real key provider, integration tests.
- Migrations row replaced the old "typo `wallter`→`wallet`" note with the actual U1–U5
  inventory.
- `Cargo.toml` row: workspace members are root + `migrations`; deps include `pasetors`,
  `async-trait`, `dotenvy` (the old "drop `macros`/`traits` crates" instruction was already
  done and removed).

### Layer Architecture diagram
- API layer now lists the auth middleware (`AuthPrincipal` injection, `src/auth/`).
- PostgreSQL box now lists the `exchange_rate_cache` table (unlogged) alongside the five
  base tables and the `purchase_items` view.

### Domain Model (ERD)
- `WALLET`: added `currency` column (U2, `CHAR(3) NOT NULL DEFAULT 'USD'`).
- `PURCHASE`: added the missing `description` column, plus the U2 columns `currency`,
  `exchange_rate` (typed `DOUBLE PRECISION` after U4 — was NUMERIC), `rate_source`,
  `rate_at`. (The ERD previously omitted `description` entirely, though the model and U1
  both have it.)
- `ITEM`: `unit_price` NUMERIC → `BIGINT` (U5); the app maps it to `i64` minor units.

### Module Structure (rewritten)
- Added `application/outbound/exchange_rate.rs`, `application/purchase/diff.rs`,
  `application/purchase_history/`, `repository/rate_cache.rs`,
  `repository/purchase_history.rs`, the `auth/` tree (`mod.rs`, `verifier.rs`,
  `paseto.rs`, `key_provider.rs`), and the empty `repository/macros.rs` placeholder.
- Removed the nonexistent `push_values helper` claim from the `repository/mod.rs` comment.
- `config/otlpconfig.rs` comment corrected: it defines **TracerConfig + MetricConfig +
  LogConfig** traits (the doc only mentioned TracerConfig + LogConfig).
- Trailing paragraph replaced: the workspace has two members (root + `migrations` refinery
  binary, built by `Dockerfile.migrate`, run by `scripts/entrypoint.sh` before serving).

### AppState and Dependency Injection
- "Current code uses `Extension(Arc<AppState>)`. Migrate to `State<...>`" → the migration
  is done; code uses `State<Arc<AppState>>`.
- `AppState` snippet corrected to the real fields: the 5-generic `purchase_service`,
  `wallet_service`, `category_service`, `item_repo`. The doc's `logger: LoggerWrapper`
  field does not exist in code and was removed.
- Added the `/{SERVICE_NAME}` route nesting (default `fin-management`) and the
  `Extension<Arc<dyn TokenVerifier>>` wiring consumed by `auth_middleware`.

### Error Handling Strategy
- `RepositoryError`: added `Serde(#[from] serde_json::Error)`.
- `ApplicationError`: replaced the speculative definition with the real one —
  `Serialization(#[from] serde_json::Error)` variant and `Internal(String)` (not
  `Internal(#[from] RepositoryError)`); the `From<RepositoryError>` impl is manual and maps
  into `Internal(e.to_string())`.
- Documented the side effect: `RepositoryError::NotFound` becomes
  `ApplicationError::Internal`, so a missing row or stale version currently surfaces as
  HTTP 500, not 404/409.
- `ApiError::From<ApplicationError>`: added the `Serialization(_) => ApiError::Internal`
  arm.

### Authentication (new section)
- Previously "Pending". Now documented: `auth_middleware` (applied via `from_fn` inside
  `api::setup_routes()`, so `/health` stays unauthenticated), `AuthPrincipal` insertion,
  `PasetoVerifier` (v4.local, `kid` in footer → `KeyProvider`; `StaticKeyProvider` with an
  empty map today), dev bypass via `AUTH_SKIP_VERIFY=1` / `AUTH_DEFAULT_USER_ID`
  (fallback `00000000-0000-0000-0000-000000000001`), and the failure status mapping
  (401 / 401 / 503 / 500).

### Repository Pattern
- `PgWalletRepo` snippet: struct field is the `PgPool` alias (not the fully-expanded bb8
  type); `row_to_wallet` now includes `currency`.
- Closing paragraph corrected: `Result<T>` in `application/inbound/repositories.rs` is
  `Result<T, ApplicationError>` — the traits speak the application error type directly;
  repos convert `RepositoryError` via `From` at the `?` site (previously described
  backwards as "maps RepositoryError via From on ApplicationError").
- Trait-syntax note qualified: repository traits use native async-fn-in-trait (with
  `#![allow(async_fn_in_trait)]`); the auth traits use `#[async_trait]` because they must
  be dyn-safe.

### Application Service Pattern
- Struct/generics rewritten to `PurchaseService<P, I, R, W, F>` with the real
  `new()` signature.
- `create()` flow rewritten to match the code: validate currency (3-letter ISO 4217) +
  items (quantity > 0, unit_price ≥ 0) → load wallet (its currency is the conversion
  target) → `resolve_rate` → `purchases.create_with_items()` (single transaction) →
  `purchase_created_total` counter. The doc's old flow (`items.bulk_create` after a plain
  `create`) is not what the code does.
- Removed the obsolete rule "purchase must have at least one item" — the code no longer
  enforces it (empty item lists are allowed).
- `CreatePurchaseCommand`: `category_id: i32` (was `u64`), added `description`,
  `currency`, `exchange_rate_override`; `purchase_date` retained.
- `UpdatePurchaseCommand`: field is `current_version` (was `version`), `category_id:
  Option<i32>`, `description` added.
- Added the item-mutation commands `AddItemsCommand`, `UpdateItemCommand`,
  `RemoveItemCommand` (all carrying `current_version`).
- Documented `ItemInput`/`ItemDto` `unit_price: i64` minor units (was `f32`).

### Currency & Exchange Rates (new section)
- Documented the resolution order implemented in `PurchaseService::resolve_rate`:
  1. `exchange_rate_override > 0` → `rate_source = "manual"`, no cache/provider call
  2. source == wallet currency → no rate stored
  3. top-of-hour cache lookup (timestamp truncated to the hour of `purchase_date` or now)
  4. cache miss → `ExchangeRateProvider::fetch_rate`, result cached
  5. provider failure → most recent cached rate of any age; none → `Internal` error
- Documented why U4/U5 exist: tokio-postgres maps `f64`/`i64` only to
  `DOUBLE PRECISION`/`BIGINT`, not NUMERIC.

### Handler Pattern
- `RequestBody` corrected: `category_id: i32`, `description`, `currency`,
  `exchange_rate_override` added.
- `ItemDto`: `unit_price: i64` (was `f32`), lives in `handlers/mod.rs` as a shared DTO.
- Handler now extracts `Extension(AuthPrincipal)` and uses `principal.user_id` — the doc's
  `user_id: uuid::Uuid::nil(), // replace with auth user` placeholder was outdated.
- Shared response DTO is `PurchaseResponse` (doc showed an ad-hoc `ResponseBody`).

### Data Flow: Create Purchase (sequence diagram rewritten)
- Now shows: wallet lookup → rate resolution (override / same-currency / cache / provider /
  stale fallback) → `create_with_items` in one BEGIN/COMMIT transaction.
- Old diagram claimed a two-step `purchases.create` + `items.bulk_create` with a multi-row
  VALUES insert — neither matches the code (`bulk_create` isn't called by the service; item
  inserts are one statement per item inside the transaction).
- Full path annotated with the `/{SERVICE_NAME}` prefix.

### Pagination and Filtering
- "Repo executes whatever it receives" removed — the repo re-applies the same defaults
  (limit 50, offset 0, start_date = now − 30 days) for `None` values.
- Documented gap: `PurchaseFilters.store` is accepted by the handler and trait but the
  repo's list SQL ignores it.

### PurchaseHistory (Audit Trail) (rewritten)
- Old text said history belongs in the service, written "in sequence" with the update and
  possibly lagging outside a transaction. Reality: the repo writes the history row inside
  the same DB transaction as the mutation (`insert_history`); version bump and history
  insert are atomic and can never lag.
- Documented the payload: `PurchaseChanges` (`application/purchase/diff.rs`), JSONB,
  tagged by `action`: `purchase_updated` (field diffs, only actually-changed fields),
  `purchase_deleted`, `item_added`, `item_updated`, `item_removed`.
- The service computes diffs and skips the repo call on no-op updates; item sequences are
  assigned in the repo (`MAX(item_sequence) + 1`), and `add_items` builds `ItemAdded`
  there.
- Optimistic-concurrency SQL updated to the real form (version bump in the UPDATE,
  `user_id` and `deleted_at` predicates). 0 rows → `RepositoryError::NotFound` →
  currently HTTP 500 (the doc previously claimed `Conflict`/409).
- `delete` also bumps `version` and records `PurchaseDeleted` in the same transaction.

### What to Build Next (rewritten)
- Removed items already done: auth middleware, request validation (now in the service
  layer), purchase-create transactions.
- Current list: real FX provider; real key provider; expose history over HTTP; fix
  NotFound→500 mapping; apply the store filter; integration tests; pagination envelope.
- Added pointer to the broken `POST /item/` route (see `docs/reports/api-review.md`).

### Observability (rewritten)
- All three OTLP signals are wired, not just "trace + log": trace layer, logs via
  `OpenTelemetryTracingBridge`, metrics via `MetricsLayer` (event fields with
  `monotonic_counter.` / `histogram.` prefixes).
- `TraceLayer` is customized: `make_span_with` (method/host/url, `otel.name`),
  `on_response` records status, emits `histogram.http_request_duration_ms` and
  `counter.http_5xx_total` for 5xx.
- Snippet corrected to the real generics (`PurchaseService<P, I, R, W, F>`) and to
  reflect that `#[instrument]` is already applied in the services, plus the
  `purchase_created_total` / `fx_provider_failure_total` counters the code emits.

### Technology Decisions
- Added rows: token verification (`pasetors`, PASETO v4.local) and dyn-safe async traits
  (`async-trait` for auth only).
- Migrations row updated: standalone binary in the workspace `migrations/` member, run by
  the container entrypoint before serving; SQL is U1–U5.

---

## 2. Verified, no change needed

- `cargo check` passes with zero code errors/warnings (claim kept).
- Route inventory implied by the doc matches `src/api/**` (`/health` unauthenticated;
  purchase, wallet, item, category sub-routers).
- `purchase_items` view columns in U2/U4/U5 match the "Currency" section text
  (`total`, `total_wallet`, currency/rate columns).
- `AppState` comment that the update_item handler re-fetches the item row via
  `state.item_repo` after `service.update_item` — accurate (`src/api/item/handlers/update_item.rs`).
- Auth failure mapping (401 missing/invalid token, 401 unknown key id, 503 key provider
  failure, 500 internal verification failure) matches `src/auth/mod.rs`.

---

## 3. Ambiguous / left as-is

These were found during the audit but were **not** "corrected" in the doc beyond pointing at
them, because resolving them requires a code decision (out of scope — no code modified):

1. **`src/repository/macros.rs` is orphaned.** The file exists but is empty (1 byte) and is
   not declared with `mod macros;` in `repository/mod.rs` — it compiles as nothing. The doc
   lists it as "(empty placeholder)". Candidate for deletion in a code pass.
2. **`POST /item/` route is broken.** `src/api/item/mod.rs` registers the create handler on
   `"/"` with no `:purchase_id` segment, but `create_item::handler` extracts
   `Path(purchase_id): Path<Uuid>` — the extraction will fail at runtime. Documented via a
   pointer to `docs/reports/api-review.md`; fixing it is a code change.
3. **`RepositoryError::NotFound` → HTTP 500.** Stale optimistic-lock versions and missing
   rows both surface as 500 instead of 404/409. Listed in "What to Build Next"; whether a
   stale version should map to 409 (Conflict) or 404 is a design decision left open.
4. **CATEGORY `id` identity not shown in the ERD.** U3 gave `category.id` an identity
   default (plain `INTEGER` before that); the ERD says `integer id PK`, which is type-correct
   but doesn't encode the identity detail. Only the migrations row in the status table
   mentions U3.
5. **`update_item` read-back bypasses the service.** The handler calls
   `state.item_repo.get(...)` directly after `service.update_item` instead of having the
   service return the item. Noted in the `AppState` comment; left open.
6. **Pseudo-signature kept.** The Repository Pattern snippet's
   `pub fn new(pool: ...) -> Self` is illustrative shorthand; the real signature is
   `new(pool: PgPool)`. Kept for brevity, consistent with the doc's style.
7. **`exchange_rate_cache.stale` flag.** U2 defines a `stale BOOL NOT NULL DEFAULT FALSE`
   column and the repo reads it, but nothing in the code sets it to `true` yet; the doc
   describes the cache but not the (currently unused) staleness flag.
8. **Stray file:** `docs/reports/ARCHITECTURE.md` — a copy of an intermediate document
   revision, apparently created by the concurrent editing session (timestamped alongside
   it). Not referenced anywhere; left untouched, but it almost certainly should be deleted
   to avoid confusion with the real `ARCHITECTURE.md` at the repo root.
