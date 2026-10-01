# REST API Review

**Scope:** `src/api/**`, the `src/auth/**` middleware it runs behind, the `src/application/**`
services it calls (`PurchaseService`, `WalletService`, `CategoryService`,
`PurchaseHistoryService`), and the `src/repository/**` SQL those services use.
**Method:** read-only static review; no code was modified.
**Base path:** `/{SERVICE_NAME}` (default `fin-management`) — `src/main.rs:78`.
**Auth:** every route is behind `auth_middleware` except `GET /health` — `src/api/mod.rs:22-27`.

---

## 1. Route inventory (as implemented)

| Method | Path | Handler | Status / notes |
|---|---|---|---|
| GET | `/health` | `check_health` | Unauthenticated; returns `pong`, no DB/readiness check (`src/api/mod.rs:24-26`) |
| GET | `/purchase/` | `list_purchases` | Filters `wallet_id`, `category_id`, `store` (ignored), `start_date`, `end_date`, `limit`, `offset`; returns bare array |
| GET | `/purchase/:id` | `get_purchase_details` | Returns purchase **without items** |
| POST | `/purchase/purchase_with_items` | `create_purchase_with_items` | Creates purchase + items; **drops `description`** |
| PUT | `/purchase/:id` | `update_purchase` | Partial update; no PATCH |
| DELETE | `/purchase/:id` | `delete_purchase` | 204; ignores `current_version` |
| GET | `/wallet/` | `list_wallets` | No pagination/filtering |
| POST | `/wallet/` | `create_wallet` | `name` only; currency not settable |
| PUT | `/wallet/:id` | `update_wallet` | Rename only |
| DELETE | `/wallet/:id` | `delete_wallet` | 204 |
| POST | `/item/` | `create_item` | **BROKEN** — route has no `:purchase_id` segment |
| PUT | `/item/:purchase_id/:sequence` | `update_item` | `current_version` in body; handler bypasses service on the read-back |
| DELETE | `/item/:purchase_id/:sequence` | `delete_item` | `current_version` in query string |
| GET | `/category/` | `list_categories` | System + caller's own |
| POST | `/category/` | `create_category` | Caller-owned |

Not present anywhere: GET by id for wallet/category, item listing, purchase history,
analytics/aggregates, OpenAPI.

---

## 2. Executive summary

The API is a thin handler layer over a reasonably well-separated service/repository stack, but the
HTTP contract has correctness, security, and observability gaps concentrated in five areas:

1. **No error taxonomy.** Every repository failure (including "row not found" and optimistic-lock
   misses) is flattened to HTTP 500 (`src/application/error.rs:19-22`). `ApiError::NotFound` and
   `Conflict` exist but are unreachable from production paths.
2. **A dead endpoint and a silent data-loss bug.** `POST /item/` can never match its handler, and
   `POST /purchase/purchase_with_items` accepts a `description` it never persists.
3. **Items are write-only.** There is no way to read items or totals after creating them.
4. **Multi-tenant integrity hole.** A purchase can be linked to another user's category because
   the create/update path never validates `category_id` against the caller.
5. **Analytics is impossible with the current surface.** No aggregate endpoints, no pagination
   envelope, no multi-value filters, no currency exposure, and a hidden 30-day default window.

Detailed, prioritized findings follow.

---

## 3. P0 — Correctness / security / data loss

### P0-1. Missing rows and version conflicts return HTTP 500, not 404/409

`From<RepositoryError> for ApplicationError` maps **all** repository errors to
`ApplicationError::Internal`:

```rust
// src/application/error.rs:19-22
impl From<RepositoryError> for ApplicationError {
    fn from(e: RepositoryError) -> Self {
        ApplicationError::Internal(e.to_string())
    }
}
```

`RepositoryError::NotFound` is the normal "no such row" signal at
`src/repository/wallet.rs:70,85,102`, `src/repository/purchase.rs:88,226,301,338,359,438,470`,
`src/repository/category.rs:77`, and `src/repository/item.rs:135,165,182`. Because it is converted
to `Internal`, it reaches `ApiError::Internal` → **500** (`src/api/mod.rs:60,72-75`).

Consequences:

- `GET /purchase/:id`, `PUT /wallet/:id`, `DELETE /purchase/:id`, etc. for a nonexistent or
  other-user id return **500** instead of **404**.
- Optimistic-lock misses (`UPDATE ... WHERE version = $n ... RETURNING *` returning 0 rows at
  `src/repository/purchase.rs:78-88,295-301,432-438,464-470`) are also **500**, even though
  `ApplicationError::Conflict`/`ApiError::Conflict` (409) exist and are never constructed
  outside the mapping table.
- Unique-constraint violations (e.g. duplicate wallet name, `migrations/migrations/U1__init.sql:8`)
  are **500** instead of **409**.
- FK violations (e.g. unknown `category_id`/`wallet_id`) are **500** instead of **400/422**.

**Fix:** map `RepositoryError::NotFound → ApplicationError::NotFound`; detect the version guard
explicitly and return `Conflict`; map SQLSTATE `23505 → Conflict`, `23503`/`23514 → Validation` in
the repository or a single error-mapping layer.

### P0-2. `POST /item/` is unreachable — route/handler path mismatch

Route registration has no purchase-id segment:

```rust
// src/api/item/mod.rs:12-13
Router::new()
    .route("/", post(handlers::create_item::handler))
```

but the handler extracts one:

```rust
// src/api/item/handlers/create_item.rs:51-55
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(purchase_id): Path<Uuid>,
    ...
```

Axum's `Path<Uuid>` extractor has no parameter to bind and rejects every request (400). Item
creation is completely dead. The route should be `/:purchase_id` (or ideally nested as
`/purchase/:id/items`).

### P0-3. `POST /purchase/purchase_with_items` silently discards `description` (data loss)

The request accepts and forwards `description`:

- `src/api/purchase/handlers/create_purchase_with_items.rs:25` (`pub description: Option<String>`)
- `:44` (`description: body.description`)
- `src/application/purchase/commands.rs:9` (`CreatePurchaseCommand.description`)

but the service never copies it into the persistence params:

```rust
// src/application/purchase/service.rs:73-84
let params = CreatePurchaseParams {
    wallet_id: cmd.wallet_id,
    store: cmd.store,
    notes: cmd.notes,
    user_id: cmd.user_id,
    category_id: cmd.category_id,
    purchase_date: cmd.purchase_date,
    currency,
    exchange_rate,
    rate_source,
    rate_at,
};  // <-- cmd.description is dropped here
```

`CreatePurchaseParams` itself has no `description` field (`src/application/inbound/repositories.rs:96-109`),
and the INSERT omits the column (`src/repository/purchase.rs:156-174`). A client that sends
`description` gets **201 with `description: null`** — silent data loss. (`PUT` does persist it, so
the two writes disagree.)

### P0-4. Items are write-only; purchase detail omits items and there is no item read endpoint

- `GET /purchase/:id` returns only the purchase — `src/api/purchase/handlers/get_purchase_details.rs:19-20`.
- `PurchaseResponse` has no `items` field — `src/api/purchase/handlers/mod.rs:11-29`.
- There is no `GET /item/...` route — `src/api/item/mod.rs:11-21`.
- `ItemRepository::list_by_purchase` exists (`src/application/inbound/repositories.rs:146`,
  `src/repository/item.rs:187-201`) but is never called anywhere in `src/**`.
- The `purchase_items` view computes `total` and `total_wallet`
  (`migrations/migrations/U5__item_price_bigint.sql:25-26`) and is never referenced from Rust.

A client can create and mutate items but can never read them or their totals. Add
`GET /purchase/:id/items` and/or embed `items`/totals in the purchase detail response.

### P0-5. Purchase can reference another user's category (cross-tenant integrity)

`PurchaseService::create` verifies wallet ownership (`src/application/purchase/service.rs:65`) but
performs no check on `category_id`; the repo INSERT passes it straight through
(`src/repository/purchase.rs:159,169`). The category FK is a plain reference
(`migrations/migrations/U1__init.sql:28`) and `CategoryRepository::get` is not user-scoped:

```rust
// src/repository/category.rs:70-80
async fn get(&self, id: i32) -> Result<Category> {
    ...
    "SELECT * FROM category WHERE id = $1"   // no user_id predicate
```

A caller who guesses another user's integer category id can attach purchases to it (and any later
join that surfaces the category name leaks it). The same gap exists on update via
`category_id` (`src/api/purchase/handlers/update_purchase.rs:26`,
`src/application/purchase/service.rs:237`). Validate that `category_id` exists and is either a
system category or owned by the caller, and scope `CategoryRepository::get` by `user_id`.

### P0-6. Production token verifier is wired to an empty key map

`build_token_verifier` constructs `StaticKeyProvider::new(HashMap::new())`
(`src/main.rs:143-145`), so any real PASETO token resolves to `KeyNotFound` and is rejected with
401 (`src/auth/paseto.rs:54-61`, `src/auth/mod.rs:60`). Outside `AUTH_SKIP_VERIFY=1`, the API is
effectively inaccessible. The local bypass (`StaticUserVerifier`, `src/auth/verifier.rs:67-78`)
accepts any token and even allows anonymous requests (`allows_anonymous` returns `true`,
`src/auth/verifier.rs:75-77`; consumed at `src/auth/mod.rs:44-53`). This is a release blocker:
wire a real key provider and make it impossible to enable `AUTH_SKIP_VERIFY` in production.

Additionally, the PASETO verifier only extracts `user_id`; it does not require or validate
`exp`, `aud`, `iss`, or scopes (`src/auth/paseto.rs:67-79`). Tokens without an expiry (or minted
for another audience) are accepted. Add required-claim and audience validation.

---

## 4. P1 — Contract / analytics gaps

### P1-1. `store` filter is accepted and silently ignored

`list_purchases` parses `store` (`src/api/purchase/handlers/list_purchases.rs:23,38`) and
`PurchaseFilters` carries it (`src/application/inbound/repositories.rs:32`), but the SQL WHERE
clause has no store predicate (`src/repository/purchase.rs:242-250`). Callers get unfiltered
results with no warning. Add the predicate (and define exact vs. partial match) or reject the
parameter.

### P1-2. Pagination has no envelope, no total, and no bounds

List endpoints return a bare JSON array (`src/api/purchase/handlers/list_purchases.rs:50-51`), so
clients cannot page reliably. `limit`/`offset` are unvalidated `i64`
(`src/api/purchase/handlers/list_purchases.rs:26-27`): a huge `limit` passes straight to SQL
(`src/repository/purchase.rs:250,257`), and a negative `offset` produces a DB error that surfaces
as 500 (see P0-1). Defaults are applied twice — in the service
(`src/application/purchase/service.rs:216-226`) and again in the repo
(`src/repository/purchase.rs:234-238`). Return `{ data, total, limit, offset }`, clamp
`limit` (e.g. 1..=200), reject negative `offset`, and count once.

### P1-3. Non-deterministic ordering makes offset pagination lossy

The list query is `ORDER BY purchase_date DESC` with no tiebreaker
(`src/repository/purchase.rs:249`). Purchases sharing a timestamp (common: `purchase_date`
defaults to `Utc::now()` at `src/repository/purchase.rs:113,148`) can appear in arbitrary order
across queries, so pages can skip or duplicate rows. Add a stable secondary key (e.g. `id`) and/or
expose a `sort` parameter.

### P1-4. Missing read endpoints that already have service/repository support

- `GET /wallet/:id` — `WalletService::get` exists (`src/application/wallet/service.rs:40-47`) but
  no route (`src/api/wallet/mod.rs:11-17`).
- `GET /category/:id` — `CategoryService::get` (`src/application/category/service.rs:47-50`) and
  repo `get` (`src/repository/category.rs:70-80`) exist but no route
  (`src/api/category/mod.rs:11-14`). Note the lookup is **not** user-scoped (P0-5).
- Purchase history — service and repo exist (`src/application/purchase_history/service.rs`,
  `src/repository/purchase_history.rs`) but the service is not in `AppState`
  (`src/main.rs:26-39`) and there is no route. The audit trail is write-only.
- Item listing — see P0-4.

### P1-5. No analytics endpoints and no read model for them

There is no aggregate endpoint (spend by category/wallet/month, date-range totals, trends), and
the only precomputed totals (`purchase_items.total`/`total_wallet`,
`migrations/migrations/U5__item_price_bigint.sql:25-26`) are never queried. Filters are
single-valued only: one `category_id`, one `wallet_id`, no multi-category, no currency filter, no
text search, no grouping parameter. Add dedicated read endpoints, e.g.
`GET /analytics/spend?from=&to=&group_by=category|wallet|month`, or at minimum expose totals and
multi-value filters. The design constraints are explored in
`docs/reports/analytics-investigation.md`.

### P1-6. Default date window silently hides data and is duplicated

`start_date` defaults to "now − 30 days" in **both** the service
(`src/application/purchase/service.rs:222-224`) and the repo
(`src/repository/purchase.rs:236-238`), so omitting the filter hides everything older than 30 days
with no indication. There is no default/max for `end_date` and no validation that
`start_date <= end_date` (a reversed range silently returns an empty list). Make the window
explicit in the response envelope and validate ordering/span.

### P1-7. Wallet currency cannot be set, and cross-currency creation is broken

`CreateWalletRequest`/`UpdateWalletRequest` accept only `name`
(`src/api/wallet/handlers/create_wallet.rs:13-16`, `src/api/wallet/handlers/update_wallet.rs:13-16`),
while the model and schema have `currency` (`src/model/wallet.rs:8`) defaulted to `'USD'`
(`migrations/migrations/U2__currency.sql:1-2`). Every wallet created through the API is USD, so
the multi-currency path is unreachable via normal use.

Worse, the FX provider wired in `main` is `MockExchangeRateProvider::new(HashMap::new())`
(`src/main.rs:56`), so any purchase whose `currency` differs from the wallet currency hits a
cache miss, then `PairNotFound`, then falls through to "no cached rate" and returns
`ApplicationError::Internal` → **500** (`src/application/purchase/service.rs:157-195`). Even if a
wallet currency could be set, non-USD purchases would fail. Accept/validate `currency` on wallet
create/update and wire a real (or seeded) rate provider.

---

## 5. P2 — Consistency, validation, DTO, hardening, tests

### P2-1. Route/verb consistency

- Singular resource names throughout (`/purchase`, `/wallet`, `/item`, `/category`;
  `src/api/*/mod.rs`); plural is the REST convention and matches the list-returns-collection
  behaviour.
- `POST /purchase/purchase_with_items` is redundant snake_case in a URL; `POST /purchase/` would
  be equivalent and consistent (`src/api/purchase/mod.rs:15-18`).
- The same path is registered with separate `.route("/:id", ...)` calls
  (`src/api/purchase/mod.rs:14,19,20`; `src/api/wallet/mod.rs:15,16`). Prefer
  `get(...).put(...).delete(...)` on one registration.
- `PUT` is used for partial updates (`update_purchase`, `update_wallet`); `PATCH` is more accurate,
  and combined with `COALESCE` (P2-2) the semantics are neither PUT nor PATCH.
- No `Location` header on 201 responses (`src/api/purchase/handlers/create_purchase_with_items.rs:54`,
  `src/api/wallet/handlers/create_wallet.rs:51`, `src/api/category/handlers/create_category.rs:49`,
  `src/api/item/handlers/create_item.rs:74`).
- `DELETE /purchase/:id` ignores `current_version` entirely and soft-deletes unconditionally
  (`src/repository/purchase.rs:314-340`), unlike every other mutation. Either require a version or
  document the difference.

### P2-2. Partial updates cannot clear optional fields

Column updates use `COALESCE($n, col)` (`src/repository/purchase.rs:280-288`,
`src/repository/item.rs:146-152`), so sending `null` for `store`/`description`/`notes`/`item_type`
is indistinguishable from omitting the field. Clients can never clear these fields. Use an
explicit "unset vs null" representation (e.g. `Option<Option<T>>` or patch operations).

### P2-3. Inconsistent optimistic-lock transport

`current_version` travels in the JSON body for create item / update item / update purchase
(`src/api/item/handlers/create_item.rs:21`, `src/api/item/handlers/update_item.rs:29`,
`src/api/purchase/handlers/update_purchase.rs:22`) but in the **query string** for delete item
(`src/api/item/handlers/delete_item.rs:20-21,28`). Query strings are logged as URLs and cannot
carry richer locks. Standardize on `If-Match`/ETag or a body field.

### P2-4. Validation gaps

- **Empty item list accepted.** `validate_items` only checks per-item fields and happily iterates an
  empty slice (`src/application/purchase/service.rs:361-377`), so `POST /purchase/purchase_with_items`
  creates a purchase with zero items. `ARCHITECTURE.md` claims items are validated non-empty.
- **Item `name` never validated** (empty/whitespace allowed) — same function.
- **No length limits** on `store`/`description`/`notes`/`name`.
- **Currency is not ISO-4217 checked.** `validate_currency` only requires 3 ASCII letters
  (`src/application/purchase/service.rs:352-359`), so `XYZ`/`AAA` are accepted. Wallet currency is
  not validated at all (P1-7).
- **`exchange_rate_override` accepts non-finite values.** The guard is `override_rate <= 0.0`
  (`src/application/purchase/service.rs:104-109`); `NaN <= 0.0` is `false`, so `NaN`/`+inf` pass
  and are stored as `DOUBLE PRECISION`. Use `is_finite() && rate > 0.0`.
- **No date-range ordering/span validation** (P1-6).
- **`category_id` not validated to exist/belong to caller** (P0-5).
- **`purchase_date`** is unvalidated (future dates, absurd past dates accepted).

### P2-5. DTO vs model leakage / layering

- `PurchaseResponse` mirrors the domain model (`src/api/purchase/handlers/mod.rs:11-53`), exposing
  `user_id` (already known from auth), `deleted_at` (always `null` on successful reads),
  `version`, and `rate_source`. It already hand-maps, so tightening it is cheap.
- `CategoryResponse` exposes `user_id`/`is_system` (`src/api/category/handlers/create_category.rs:12-21`)
  — internal tenancy metadata.
- Wallet `ResponseBody` exposes `user_id`/`deleted_at`
  (`src/api/wallet/handlers/create_wallet.rs:18-26`).
- `update_item` bypasses the service and reads `state.item_repo` directly after the mutation
  (`src/api/item/handlers/update_item.rs:55-59`), violating the thin-handler rule. The service
  returns the `Purchase`, not the changed `Item`, forcing the handler into the repo.
- No request DTO has `#[serde(deny_unknown_fields)]` (verified by grep), so misspelled fields are
  silently ignored — e.g. `description` on create is doubly silent once P0-3 is considered.

### P2-6. Error envelope is inconsistent for extractor/auth failures

Only `ApiError` produces the `{"error": "..."}` JSON shape (`src/api/mod.rs:62-75`). Axum's
built-in `Json`/`Query`/`Path` rejections (malformed body, bad UUID, missing field, bad query
type) return default plain-text responses, and the auth middleware returns `(StatusCode, String)`
tuples (`src/auth/mod.rs:32-71`) such as `Missing Authorization header`. 401 responses also omit
`WWW-Authenticate`. This makes the API's error contract impossible to describe reliably in a
generated client. Add custom rejections and a body-producing auth error so all errors share one
contract.

### P2-7. Request/ops hardening absent

No `CorsLayer`, no explicit `DefaultBodyLimit`, and no `deny_unknown_fields` (verified by grep over
`src` and `Cargo.toml`). The default 2 MB body limit may be acceptable but should be a deliberate
choice. `GET /health` (`src/api/mod.rs:24-26`) does not check the DB, so it cannot serve as a
readiness probe (a liveness/readiness split is missing).

### P2-8. Soft-delete vs unique constraint

`wallet` has `UNIQUE(user_id, name)` (`migrations/migrations/U1__init.sql:8`) that ignores
`deleted_at`, so a name cannot be reused after soft-delete, and the resulting unique violation is
returned as 500 (P0-1). Use a partial unique index on `deleted_at IS NULL` (or handle it as 409).

### P2-9. Tests: no API-layer, integration, auth, or repository coverage

There are 21 tests total, all unit tests in `src/application/purchase/service.rs:379-925` and
`src/application/purchase/diff.rs:124-287` (mock repos; several traits are `todo!()`). There are
**no** tests under `src/api`, no router/HTTP integration tests, no DB-backed tests, no auth
middleware/verifier tests, and no repository SQL tests (`tests/` does not exist). The two most
severe defects (P0-1 status mapping, P0-2 dead item route) would be caught immediately by a single
end-to-end smoke test. Recommended: `tower::ServiceExt::oneshot` router tests asserting status
codes per resource, a test-Postgres suite running the migrations, auth tests for missing/invalid
tokens, and a schema/spec conformance test.

### P2-10. No OpenAPI / machine-readable contract

There is no `utoipa`/OpenAPI/Swagger dependency or route (grep over `src`, `Cargo.toml`,
migrations, and yaml/json is empty), and no spec under `docs/`. The only contract is the prose
sketch in `ARCHITECTURE.md`. Add `utoipa` + `utoipa-swagger-ui` (or a checked-in `openapi.yaml`)
generated from the DTOs and validate it in CI so the documented contract cannot drift.

---

## 6. Money & currency handling (dedicated review)

This area deserves extra scrutiny because it is the core domain.

- **Minor-unit convention.** `item.unit_price` is `BIGINT` interpreted as i64 minor units
  (`src/model/item.rs:8`; `migrations/migrations/U5__item_price_bigint.sql:6-7`;
  `src/repository/item.rs:20-28`). Good, but the **currency exponent is not modelled** — JPY (0
  decimals), KWD (3 decimals) etc. cannot be represented correctly, and there is no field or
  endpoint telling a client what exponent applies. Add per-currency exponent handling (or store
  amounts as decimal with explicit scale).
- **Rates are floating point.** `purchase.exchange_rate` and `exchange_rate_cache.rate` are
  `DOUBLE PRECISION` (`migrations/migrations/U4__rate_double.sql:8-12`) and the app binds `f64`
  (`src/application/inbound/repositories.rs:106`; `src/application/outbound/exchange_rate.rs:28`).
  Monetary conversion via binary floating point loses precision. Prefer a decimal type / rational
  or integer-scaled rate with explicit rounding.
- **Silent 1:1 fallback.** The view computes
  `sum(...) * COALESCE(p.exchange_rate, 1)` (`migrations/migrations/U5__item_price_bigint.sql:26`).
  A missing rate is silently treated as parity rather than flagged, which can under/over-state
  wallet totals. This is especially risky given P1-7.
- **Override validation is weak.** Only `<= 0.0` is rejected; `NaN`/`Inf` pass (P2-4), and there is
  no sanity band against the market rate even though the product prompt asks for one
  (`prompts/multi-currency.md:38-42`).
- **Rate metadata is exposed but not explained.** `PurchaseResponse` returns `exchange_rate`,
  `rate_source`, `rate_at`, `currency` (`src/api/purchase/handlers/mod.rs:22-25`) but there is no
  documented rounding rule, no effective wallet amount in the response, and no way to see
  `total`/`total_wallet` (P0-4).
- **Wallet currency is immutable/always USD through the API** (P1-7), so the entire conversion
  pipeline is unreachable by real clients.

---

## 7. Suggested fix order

1. **P0-1** error mapping (fixes wrong 404/409/422 across the whole API) and **P0-2** item route.
2. **P0-3** persist `description` on create; **P0-4** expose items/totals; **P0-5** scope
   `category_id`.
3. **P0-6** real key provider + required PASETO claims; block the dev bypass in production.
4. **P1-1** `store` predicate; **P1-2/P1-3** pagination envelope, bounds, stable sort;
   **P1-6** explicit date-window semantics.
5. **P1-4/P1-5** missing read/analytics endpoints; **P1-7** wallet currency + real FX provider.
6. **P2** validation (empty items, finite rates, ISO-4217, lengths), DTO tightening, consistent
   error envelope, standardized locking, hardening, tests, OpenAPI.
