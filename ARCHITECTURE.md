# Architecture: financial-management

Monolith. Axum. Postgres. Four layers. No microservices — domain too small, team too small.

---

## Implementation Status

| Path | Status | Notes |
|---|---|---|
| `src/config/` | Done | OTLP (traces + logs + metrics) + Postgres config, bb8 pool setup, connection string |
| `src/model/` | Done | All structs match migrated schema — currency columns (U2), BIGINT item prices (U5) |
| `src/application/error.rs` | Done | `ApplicationError` enum; `From<RepositoryError>` maps `NotFound`→`NotFound`, `VersionConflict`→`Conflict`, everything else→`Internal(String)` |
| `src/application/inbound/repositories.rs` | Done | Traits: Wallet, Purchase, Item, Category, PurchaseHistory, RateCache, UserProfile, Analytics — plus params/filter structs, `RateUpdate`, `UserProfileUpdate`/`UserProfileValues`, `PurchaseFilters::normalized` |
| `src/application/purchase/service.rs` | Done | `PurchaseService<P,I,R,W,F>` — create (currency + rate resolution), get, list, update, add_items, update_item, remove_item, delete |
| `src/application/purchase/diff.rs` | Done | `PurchaseChanges` — JSONB history payloads, field + item diffs |
| `src/application/wallet/service.rs` | Done | `WalletService<W>` — create, list, get, update, delete |
| `src/application/category/service.rs` | Done | `CategoryService<C>` — create, list, get |
| `src/application/purchase_history/service.rs` | Done | `PurchaseHistoryService<P>` — list_for_purchase (not yet wired into routes) |
| `src/application/outbound/exchange_rate.rs` | Done | `ExchangeRateProvider` trait + `MockExchangeRateProvider` |
| `src/repository/mod.rs` | Done | `RepositoryError` (DbError, PoolError, NotFound, VersionConflict, Serde), `PgPool` type alias |
| `src/repository/purchase.rs` | Done | Transactions: purchase+items insert, version bump + history insert in one tx |
| `src/repository/item.rs` | Done | `PgItemRepo` — per-purchase sequence allocation, unit_price as BIGINT minor units |
| `src/repository/wallet.rs` | Done | `PgWalletRepo` — soft-delete via `deleted_at` |
| `src/repository/category.rs` | Done | `PgCategoryRepo` — system + user categories |
| `src/repository/rate_cache.rs` | Done | `PgRateCacheRepo` — top-of-hour cache over unlogged `exchange_rate_cache` |
| `src/repository/purchase_history.rs` | Done | `PgPurchaseHistoryRepo` — read side only (writes happen inside purchase tx) |
| `src/auth/` | Done | PASETO v4.local verifier, `TokenVerifier` trait, key provider abstraction, auth middleware |
| `src/api/mod.rs` | Done | `ApiError` with `From<ApplicationError>`, `setup_routes()`, `/health` endpoint |
| `src/api/purchase/` | Done | All 5 handlers wired |
| `src/api/item/` | Done | create, update, delete handlers |
| `src/api/wallet/` | Done | create, list, update, delete handlers |
| `src/api/category/` | Done | create, list handlers |
| `src/lib.rs` | Done | Library surface shared by the binary and the integration tests: `AppState`, `build_app_state`, `build_router` (the single assembly path), `connect_pool`, `build_token_verifier`, `drain_projection` |
| `src/main.rs` | Done | Thin binary: `.env`, `config::init` (tracing + pool), maintenance commands (`--backfill-facts`, `--reconcile-facts`, `--report-stuck-outbox`, `--purge-outbox`), background listener + projector, `build_router`, serve :8080 |
| `src/application/currency.rs` | Done | ISO-4217 shape validation, scaled-rate helpers (`RATE_SCALE`, `rate_to_scaled`, `scaled_to_rate`, `convert_minor`), minor-unit exponents (`minor_unit`, `format_minor`) |
| `src/application/analytics/` | Done | `FactProjector` (generic over `AnalyticsRepository`), `maintenance` (purge + stuck-outbox report) |
| `src/application/user_profile/` | Done | `UserProfileService` — get (synthesised default), partial update |
| `src/repository/analytics.rs` | Done | `PgAnalyticsRepo` implementing `AnalyticsRepository`; outbox claim, fact rebuild, purge, stuck-outbox, lag, reconcile; plus `listen_forever` (LISTEN/NOTIFY transport) |
| `src/repository/user_profile.rs` | Done | `PgUserProfileRepo` — get + upsert |
| `tests/` | Done | Database-backed integration tests driven through the HTTP router against a real TimescaleDB |
| `scripts/` | Done | `entrypoint.sh`, `validate-observability.sh`, `test-db.sh` |
| `Cargo.toml` | Done | Workspace members: root + `migrations` (refinery embed binary); deps incl. `pasetors`, `async-trait`, `dotenvy` |
| Migrations U1–U15 | Done | U1 init, U2 currency + unlogged rate cache, U3 category identity, U4 rate → double precision, U5 item price → bigint, U6 TimescaleDB extension, U7 user_profile, U8 nullable category + classification_status, U9 merchant, U10 rates → scaled bigint, U11 analytics outbox + job queue, U12 purchase_fact hypertable, U13 unresolved rates (no parity), U14 currency reference (minor units), U15 conversion accounts for exponents |
| Real FX provider | Pending | `MockExchangeRateProvider` in use |
| Real key provider | Pending | `StaticKeyProvider` (empty map) in use |
| Integration tests | Done | 53 tests: unit tests with mock repos, a fake `AnalyticsRepository` driving the projector, and database-backed tests through the HTTP router |

`cargo check` passes with zero errors and zero warnings.

---

## Layer Architecture

```
┌─────────────────────────────────────────────────────┐
│                    HTTP Clients                      │
└────────────────────────┬────────────────────────────┘
                         │ HTTP
┌────────────────────────▼────────────────────────────┐
│                   API Layer                          │
│   src/api/                                           │
│   • Axum handlers — parse HTTP, call application     │
│   • Auth middleware — verifies tokens, injects       │
│     AuthPrincipal (src/auth/)                        │
│   • Request/Response DTOs (serde Deserialize)        │
│   • ApiError → HTTP status mapping                   │
│   • Route registration                               │
└────────────────────────┬────────────────────────────┘
                         │ Commands / Queries
┌────────────────────────▼────────────────────────────┐
│                Application Layer                     │
│   src/application/                                   │
│   • Use-case structs (PurchaseService, etc.)         │
│   • Owns business logic — validation, orchestration  │
│   • Calls repository traits (never concrete impls)   │
│   • Returns domain models or ApplicationError        │
└────────────────────────┬────────────────────────────┘
                         │ Repository Traits
┌────────────────────────▼────────────────────────────┐
│                Repository Layer                      │
│   src/repository/                                    │
│   • Implements traits from application/inbound/      │
│   • Raw SQL via tokio-postgres                       │
│   • Maps Row → domain model                          │
│   • Returns RepositoryError                          │
└────────────────────────┬────────────────────────────┘
                         │ SQL
┌────────────────────────▼────────────────────────────┐
│                  PostgreSQL                          │
│   Tables: wallet, category, purchase, item,          │
│           purchase_history, exchange_rate_cache      │
│           (unlogged)                                 │
│   Views:  purchase_items                             │
└─────────────────────────────────────────────────────┘
```

---

## Domain Model

```mermaid
erDiagram
    WALLET {
        uuid id PK
        uuid user_id
        varchar name
        char currency
        timestamptz created_at
        timestamptz updated_at
        timestamptz deleted_at
    }

    CATEGORY {
        integer id PK
        varchar name
        text description
        uuid user_id
        bool is_system
        timestamptz created_at
        timestamptz updated_at
    }

    PURCHASE {
        uuid id PK
        uuid user_id
        uuid wallet_id FK
        integer category_id FK
        varchar store
        text description
        text notes
        int version
        char currency
        bigint exchange_rate_scaled       -- rate * 1e9 (U10)
        varchar rate_source               -- manual | api | unresolved
        timestamptz rate_at
        integer merchant_id FK            -- nullable (U9)
        varchar classification_status     -- unclassified | suggested | confirmed (U8)
        timestamptz purchase_date
        timestamptz deleted_at
        timestamptz created_at
        timestamptz updated_at
    }

    USER_PROFILE {
        uuid user_id PK
        text timezone                     -- IANA name
        char base_currency                -- default for new wallets only
        text locale
    }

    CURRENCY {
        char code PK
        smallint minor_unit               -- 0 for JPY, 2 for USD, 3 for KWD (U14)
        text name
    }

    MERCHANT {
        uuid id PK
        uuid user_id
        text name
        text normalized_name
    }

    ITEM {
        integer item_sequence PK
        uuid purchase_id PK,FK
        varchar name
        bigint unit_price
        integer quantity
        varchar item_type
    }

    PURCHASE_HISTORY {
        uuid id PK
        uuid purchase_id FK
        int version
        jsonb changes
        timestamptz changed_at
    }

    WALLET ||--o{ PURCHASE : "funds"
    CATEGORY ||--o{ PURCHASE : "classifies"
    PURCHASE ||--o{ ITEM : "contains"
    PURCHASE ||--o{ PURCHASE_HISTORY : "tracks changes"
```

---

## Module Structure

```
src/
├── lib.rs                           # library surface: AppState, build_router (shared by server + tests)
├── main.rs                          # binary: boot, maintenance commands, serve :8080
├── tests/
│   └── common/mod.rs                # integration-test harness (pool, header user verifier, helpers)
├── scripts/                         # entrypoint.sh, test-db.sh, validate-observability.sh
├── config/
│   ├── mod.rs                       # Config struct, env var helpers, tracing stack init
│   ├── otlpconfig.rs                # TracerConfig + MetricConfig + LogConfig traits + impl
│   └── postgresconfig.rs            # bb8 pool setup
│
├── model/                           # Pure domain structs — no DB, no HTTP
│   ├── mod.rs
│   ├── wallet.rs
│   ├── category.rs
│   ├── purchase.rs
│   ├── item.rs
│   └── purchase_history.rs
│
├── application/
│   ├── mod.rs
│   ├── error.rs                     # ApplicationError enum
│   ├── inbound/
│   │   ├── mod.rs
│   │   └── repositories.rs          # Repository traits + param/filter structs
│   ├── outbound/
│   │   ├── mod.rs
│   │   └── exchange_rate.rs         # ExchangeRateProvider trait + MockExchangeRateProvider
│   ├── purchase/
│   │   ├── mod.rs
│   │   ├── service.rs               # PurchaseService struct + use cases (+ unit tests)
│   │   ├── commands.rs              # Create/Update/AddItems/UpdateItem/RemoveItem commands
│   │   └── diff.rs                  # PurchaseChanges — JSONB history payloads
│   ├── wallet/
│   │   ├── mod.rs
│   │   └── service.rs
│   ├── category/
│   │   ├── mod.rs
│   │   └── service.rs
│   ├── user_profile/
│   │   ├── mod.rs
│   │   └── service.rs               # get + partial update, timezone/base-currency validation
│   ├── analytics/
│   │   ├── mod.rs
│   │   ├── projector.rs             # FactProjector<R: AnalyticsRepository> (+ fake-repo tests)
│   │   └── maintenance.rs           # purge + stuck-outbox report
│   └── purchase_history/
│       ├── mod.rs
│       └── service.rs               # read-side history queries (not yet routed)
│
├── repository/
│   ├── mod.rs                       # RepositoryError, PgPool alias
│   ├── purchase.rs                  # impl PurchaseRepository for PgPurchaseRepo
│   ├── item.rs                      # impl ItemRepository for PgItemRepo
│   ├── wallet.rs                    # impl WalletRepository for PgWalletRepo
│   ├── category.rs                  # impl CategoryRepository for PgCategoryRepo
│   ├── purchase_history.rs          # impl PurchaseHistoryRepository for PgPurchaseHistoryRepo
│   ├── rate_cache.rs                # impl RateCacheRepository for PgRateCacheRepo
│   ├── user_profile.rs              # impl UserProfileRepository for PgUserProfileRepo
│   ├── analytics.rs                 # impl AnalyticsRepository for PgAnalyticsRepo + LISTEN/NOTIFY
│   └── macros.rs                    # (empty placeholder)
│
├── auth/
│   ├── mod.rs                       # auth_middleware + AuthPrincipal
│   ├── verifier.rs                  # TokenVerifier trait + StaticUserVerifier (dev bypass)
│   ├── paseto.rs                    # PASETO v4.local verifier
│   └── key_provider.rs              # KeyProvider trait + StaticKeyProvider
│
└── api/
    ├── mod.rs                       # setup_routes(), ApiError, error mapping, /health
    ├── purchase/
    │   ├── mod.rs                   # setup_routes() for /purchase
    │   └── handlers/
    │       ├── mod.rs               # shared DTOs for purchase API
    │       ├── create_purchase_with_items.rs
    │       ├── get_purchase_details.rs
    │       ├── list_purchases.rs
    │       ├── update_purchase.rs
    │       └── delete_purchase.rs
    ├── item/
    │   ├── mod.rs
    │   └── handlers/
    │       ├── create_item.rs
    │       ├── update_item.rs
    │       └── delete_item.rs
    ├── wallet/
    │   ├── mod.rs
    │   └── handlers/
    │       ├── create_wallet.rs
    │       ├── list_wallets.rs
    │       ├── update_wallet.rs
    │       └── delete_wallet.rs
    └── category/
        ├── mod.rs
        └── handlers/
            ├── create_category.rs
            └── list_categories.rs
```

The cargo workspace has two members: the root binary and `migrations/` — a standalone refinery binary that embeds the SQL files and runs them (`Dockerfile.migrate` builds it).

---

## AppState and Dependency Injection

Code uses `axum::extract::State<Arc<AppState>>` — typesafe, catches missing state at compile time not runtime panic.

**AppState (src/main.rs):**

```rust
pub struct AppState {
    pub purchase_service: Arc<
        PurchaseService<
            PgPurchaseRepo,
            PgItemRepo,
            PgRateCacheRepo,
            PgWalletRepo,
            MockExchangeRateProvider,
        >,
    >,
    pub wallet_service:   Arc<WalletService<PgWalletRepo>>,
    pub category_service: Arc<CategoryService<PgCategoryRepo>>,
    pub item_repo:        Arc<PgItemRepo>, // update_item handler re-fetches the row after service.update_item
}
```

Routes are nested under `/{SERVICE_NAME}` (default `fin-management`). The token verifier is attached as an `Extension<Arc<dyn TokenVerifier>>` layer and consumed by `auth_middleware`.

Handlers extract state and principal, call only needed service:

```rust
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let purchase = state.purchase_service.create(cmd).await?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}
```

Split into per-domain extension layers if compile times become problem. At this scale, one `AppState` fine.

---

## Error Handling Strategy

Three error types. Each layer owns its own. Conversions flow upward.

```
RepositoryError  →  ApplicationError  →  ApiError
(tokio-postgres)     (business logic)     (HTTP status)
```

**Repository error** — wraps DB failures:

```rust
// src/repository/mod.rs
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Database error: {0}")]
    DbError(#[from] tokio_postgres::Error),
    #[error("Pool error: {0}")]
    PoolError(#[from] bb8_postgres::bb8::RunError<tokio_postgres::Error>),
    #[error("Row not found")]
    NotFound,
    #[error("Version conflict")]
    VersionConflict,
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
```

**Application error** — lives in `application/error.rs`:

```rust
// src/application/error.rs
#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<RepositoryError> for ApplicationError {
    fn from(e: RepositoryError) -> Self {
        match e {
            RepositoryError::NotFound => ApplicationError::NotFound("resource not found".into()),
            RepositoryError::VersionConflict => ApplicationError::Conflict(
                "version mismatch: the resource was modified; reload and retry".into(),
            ),
            other => ApplicationError::Internal(other.to_string()),
        }
    }
}
```

`From<RepositoryError>` on `ApplicationError` means repository calls use `?` directly — `NotFound` surfaces as HTTP 404 and `VersionConflict` (stale optimistic-lock version) as HTTP 409; all other repository errors are 500.

**API error** — maps to HTTP:

```rust
// src/api/mod.rs
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("{0}")]
    BadRequest(String),
    #[error("Internal server error")]
    Internal,
}

impl From<ApplicationError> for ApiError {
    fn from(e: ApplicationError) -> Self {
        match e {
            ApplicationError::NotFound(msg)   => ApiError::NotFound(msg),
            ApplicationError::Conflict(msg)   => ApiError::Conflict(msg),
            ApplicationError::Unauthorized    => ApiError::Unauthorized,
            ApplicationError::Validation(msg) => ApiError::BadRequest(msg),
            ApplicationError::Serialization(_) => ApiError::Internal,
            ApplicationError::Internal(_)     => ApiError::Internal,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ApiError::NotFound(msg)  => (StatusCode::NOT_FOUND, msg.clone()),
            ApiError::Conflict(msg)  => (StatusCode::CONFLICT, msg.clone()),
            ApiError::Unauthorized   => (StatusCode::UNAUTHORIZED, "Unauthorized".into()),
            ApiError::BadRequest(msg)=> (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::Internal       => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into()),
        };
        (status, Json(json!({ "error": body }))).into_response()
    }
}
```

Handlers return `Result<_, ApiError>`, use `?` on `ApplicationError` — `From` impl does conversion.

---

## Authentication

Token-based, no sessions. `auth_middleware` (`src/auth/mod.rs`, applied via `from_fn` inside `api::setup_routes()`) reads the `Authorization` header and delegates to a `TokenVerifier` injected from `main.rs` as `Extension<Arc<dyn TokenVerifier>>`. On success it inserts `AuthPrincipal { user_id }` into the request extensions; handlers extract `Extension<AuthPrincipal>`.

- Default verifier: `PasetoVerifier` — PASETO v4.local. The token footer carries a `kid`, resolved to a symmetric key via `KeyProvider` (an external key service owns key material and rotation; today a `StaticKeyProvider` with an empty map, pending the real service).
- Dev bypass: `AUTH_SKIP_VERIFY=1` → `StaticUserVerifier`, which accepts any token (or a missing header) and authenticates as `AUTH_DEFAULT_USER_ID`, defaulting to `00000000-0000-0000-0000-000000000001`.

Failure mapping: missing/invalid token → 401, unknown key id → 401, key provider failure → 503, internal verification failure → 500.

---

## Repository Pattern

Repository traits in `application/inbound/repositories.rs`. Concrete impls in `src/repository/`.

Trait uses async-in-trait syntax (Rust 1.75+) — keep. No `async_trait` macro needed (the `auth/` traits do use `#[async_trait]`; repository traits rely on `#![allow(async_fn_in_trait)]` instead).

**Pattern for one implementation:**

```rust
// src/repository/wallet.rs
use crate::application::inbound::repositories::{WalletRepository, Result};
use crate::model::wallet::Wallet;
use super::{RepositoryError, PgPool};

pub struct PgWalletRepo {
    pool: PgPool,
}

impl PgWalletRepo {
    pub fn new(pool: ...) -> Self { Self { pool } }
}

impl WalletRepository for PgWalletRepo {
    async fn create(&self, user_id: Uuid, name: String) -> Result<Wallet> {
        let conn = self.pool.get().await
            .map_err(|e| RepositoryError::PoolError(e))?;

        let row = conn
            .query_one(
                "INSERT INTO wallet (id, user_id, name) VALUES ($1, $2, $3) RETURNING *",
                &[&Uuid::new_v4(), &user_id, &name],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_wallet(&row))
    }
    // ...
}

fn row_to_wallet(row: &tokio_postgres::Row) -> Wallet {
    Wallet {
        id:         row.get("id"),
        user_id:    row.get("user_id"),
        name:       row.get("name"),
        currency:   row.get("currency"), // default 'USD' from migration U2
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}
```

`Result<T>` type alias in `application/inbound/repositories.rs` is `Result<T, ApplicationError>`. Repo impls return `RepositoryError` internally and convert via `From` at the `?` site — no manual mapping. (`NotFound` → 404, `VersionConflict` → 409.)

---

## Application Service Pattern

Services own use-case logic. Receive command/query structs, call repos, enforce rules.

```rust
// src/application/purchase/service.rs
use crate::application::inbound::repositories::{
    ItemRepository, PurchaseRepository, RateCacheRepository, WalletRepository,
};
use crate::application::outbound::exchange_rate::ExchangeRateProvider;
use crate::application::error::ApplicationError;
use crate::model::purchase::Purchase;

pub struct PurchaseService<P, I, R, W, F>
where
    P: PurchaseRepository,
    I: ItemRepository,
    R: RateCacheRepository,
    W: WalletRepository,
    F: ExchangeRateProvider,
{
    purchases: P,
    items:     I,
    rate_cache: R,
    wallets:    W,
    fx:         F,
}

impl<P, I, R, W, F> PurchaseService<P, I, R, W, F>
where
    P: PurchaseRepository,
    I: ItemRepository,
    R: RateCacheRepository,
    W: WalletRepository,
    F: ExchangeRateProvider,
{
    pub fn new(purchases: P, items: I, rate_cache: R, wallets: W, fx: F) -> Self {
        Self { purchases, items, rate_cache, wallets, fx }
    }

    pub async fn create(
        &self,
        cmd: CreatePurchaseCommand,
    ) -> Result<Purchase, ApplicationError> {
        // 1. validate currency (3-letter ISO 4217) + items (quantity > 0, unit_price >= 0)
        let currency = cmd.currency.to_uppercase();
        validate_currency(&currency)?;
        validate_items(&cmd.items)?;

        // 2. load wallet — defines the holding currency
        let wallet = self.wallets.get(cmd.user_id, cmd.wallet_id).await?;

        // 3. resolve exchange rate (see Currency & Exchange Rates)
        let (exchange_rate, rate_source, rate_at) = self
            .resolve_rate(&cmd, &currency, &wallet.currency, cmd.purchase_date)
            .await?;

        // 4. atomic persist — purchase + items in one transaction
        let params = CreatePurchaseParams { /* fields from cmd + rate */ };
        let (purchase, _items) = self.purchases.create_with_items(params, cmd.items).await?;

        tracing::info!(monotonic_counter.purchase_created_total = 1);
        Ok(purchase)
    }

    // also: get, list, update (diff-based), add_items, update_item,
    // remove_item, delete — all call repos, all #![instrument]-ed
}
```

```rust
// src/application/purchase/commands.rs
use crate::application::inbound::repositories::ItemInput;

pub struct CreatePurchaseCommand {
    pub user_id:                uuid::Uuid,
    pub wallet_id:              uuid::Uuid,
    pub category_id:            i32,
    pub store:                  Option<String>,
    pub description:            Option<String>,
    pub notes:                  Option<String>,
    pub currency:               String,
    pub exchange_rate_override: Option<f64>,
    pub purchase_date:          Option<chrono::DateTime<chrono::Utc>>,
    pub items:                  Vec<ItemInput>,
}

pub struct UpdatePurchaseCommand {
    pub id:              uuid::Uuid,
    pub user_id:         uuid::Uuid,
    pub current_version: i32,
    pub store:           Option<String>,
    pub description:     Option<String>,
    pub notes:           Option<String>,
    pub category_id:     Option<i32>,
}

pub struct AddItemsCommand  { id, user_id, current_version, items: Vec<ItemInput> }
pub struct UpdateItemCommand { id, user_id, current_version, sequence, name, unit_price: Option<i64>, quantity: Option<i32>, item_type }
pub struct RemoveItemCommand { id, user_id, current_version, sequence }
```

`ItemInput` carries `unit_price: i64` — prices are minor units (e.g. cents), matching the BIGINT `item.unit_price` column.

---

## Currency & Exchange Rates

Purchases carry their own currency (`char(3)`), independent of the wallet's holding currency. On create, the service resolves the rate (wallet currency = target):

1. `exchange_rate_override` > 0 wins — `rate_source = "manual"`, no cache or provider call
2. source currency == wallet currency → no rate stored
3. top-of-hour cache lookup (of `purchase_date`, or now) in `exchange_rate_cache`
4. cache miss → `ExchangeRateProvider.fetch_rate()` → result cached
5. provider failure → fall back to most recent cached rate of any age; if there is none the Purchase is still **recorded** with no conversion, marked `rate_source = "unresolved"` (ADR-0006). A failed insert would block bookkeeping on a third-party outage.

Every purchase stores `currency`, `exchange_rate_scaled`, `rate_source`, `rate_at` so totals can be recomputed. The `purchase_items` view exposes `total` (source currency), `total_wallet` (wallet currency, or NULL when unresolved) and `conversion_resolved`.

**Representation.** Prices are integers in minor units; rates are scaled integers (`rate * 1e9`), never floating point in storage or aggregation (ADR/`docs/MONEY.md`). Tokio-postgres cannot bind NUMERIC, which is why U4/U5 originally moved to `DOUBLE PRECISION` and U10 moved rates to `BIGINT` instead.

**Exponents.** The number of decimal places is per currency (JPY 0, USD 2, KWD 3), held in the `currency` reference table (U14). Conversion must apply the exponent ratio as well as the rate:

    amount_minor_target = amount_minor_source * rate * 10^(target_exp - source_exp)

Omitting the ratio is a silent 100x error, which is why U15 added it to the view.

**Never invent parity.** A missing rate yields no converted amount at all; 1:1 substitution is never applied.

**Reporting scope.** Amounts are never summed across currencies: within a wallet figures use that wallet's own currency, and `user_profile.base_currency` only defaults newly created wallets (ADR-0002).

---

## Handler Pattern (API Layer)

Handlers thin. Parse input, call service, map result. No business logic.

```rust
// src/api/purchase/handlers/create_purchase_with_items.rs
use axum::{
    extract::{State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError,
    application::purchase::commands::CreatePurchaseCommand,
    auth::AuthPrincipal,
    AppState,
};

use super::{ItemDto, PurchaseResponse};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub wallet_id:     Uuid,
    pub category_id:   i32,
    pub store:         Option<String>,
    pub description:   Option<String>,
    pub notes:         Option<String>,
    pub currency:      String,
    #[serde(default)]
    pub exchange_rate_override: Option<f64>,
    pub purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    pub items:         Vec<ItemDto>,
}

// ItemDto (in handlers/mod.rs): name: String, unit_price: i64,
// quantity: i32, item_type: Option<String> — converts Into ItemInput

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cmd = CreatePurchaseCommand {
        user_id:       principal.user_id, // from auth middleware
        wallet_id:     body.wallet_id,
        category_id:   body.category_id,
        store:         body.store,
        description:   body.description,
        notes:         body.notes,
        currency:      body.currency,
        exchange_rate_override: body.exchange_rate_override,
        purchase_date: body.purchase_date,
        items:         body.items.into_iter().map(Into::into).collect(),
    };

    let purchase = state.purchase_service.create(cmd).await?;

    Ok((StatusCode::CREATED, Json(PurchaseResponse::from(purchase))).into_response())
}
```

`?` on `state.purchase_service.create(cmd).await?` works because `ApiError: From<ApplicationError>`.

---

## Data Flow: Create Purchase

```mermaid
sequenceDiagram
    participant Client
    participant Handler as API Handler<br/>create_purchase_with_items
    participant Service as PurchaseService
    participant WalletRepo as PgWalletRepo
    participant Rate as RateCacheRepo / FX provider
    participant PurchaseRepo as PgPurchaseRepo
    participant DB as PostgreSQL

    Client->>Handler: POST /{service_name}/purchase/purchase_with_items
    Handler->>Handler: JSON → CreatePurchaseCommand<br/>(user_id from AuthPrincipal)
    Handler->>Service: service.create(cmd)
    Service->>Service: validate currency + items
    Service->>WalletRepo: get(user_id, wallet_id)
    WalletRepo-->>Service: Wallet (holding currency)
    Service->>Service: resolve rate — override | same-currency | cache | provider | stale fallback
    Service->>PurchaseRepo: create_with_items(params, items)
    PurchaseRepo->>DB: BEGIN<br/>INSERT INTO purchase ...<br/>INSERT INTO item (seq per item)<br/>COMMIT
    DB-->>PurchaseRepo: Purchase + Vec<Item>
    PurchaseRepo-->>Service: Ok((Purchase, Vec<Item>))
    Service-->>Handler: Ok(Purchase)
    Handler-->>Client: 201 Created { ...PurchaseResponse }
```

Routes are nested under `/{SERVICE_NAME}` (default `fin-management`), so the full path is e.g. `/fin-management/purchase/purchase_with_items`.

---

## Pagination and Filtering

Defaults, bounds and the default window are defined once, in
`PurchaseFilters::normalized` (`DEFAULT_PAGE_LIMIT`, `MAX_PAGE_LIMIT`,
`DEFAULT_WINDOW_DAYS`). The API, service and repository layers all call that same
idempotent function, so they cannot disagree about what an absent `limit` means or
how large a page may be.

`PurchaseFilters` in `application/inbound/repositories.rs` good. Default `limit`/`offset` in service layer, not repo. Repo executes whatever it receives.

Service defaults:
- `limit`: 50
- `offset`: 0
- `start_date`: 30 days ago if not provided

(The repo re-applies the same defaults if called directly.) Note: `PurchaseFilters.store` is accepted at the API and trait level but the repo's list SQL does not apply it yet — see audit report.

---

## PurchaseHistory (Audit Trail)

Every purchase mutation writes one row to `purchase_history` inside the same DB transaction as the mutation itself — the version bump and the history insert are atomic. Nothing lags.

The payload is `PurchaseChanges` (`application/purchase/diff.rs`), serialized to JSONB and tagged by `action`:

- `PurchaseUpdated { fields: { store|description|notes|category_id: {from, to} } }` — service computes the diff first; a no-op update bumps nothing and writes nothing
- `PurchaseDeleted`
- `ItemAdded { items: [ItemSnapshot] }`
- `ItemUpdated { items: [{ item_sequence, from, to }] }`
- `ItemRemoved { items: [ItemSnapshot] }`

The service builds the change payload; the repo persists it (`insert_history` inside the transaction). Reads go through `PurchaseHistoryRepository::get_for_purchase` / `PurchaseHistoryService::list_for_purchase` — not yet exposed via an API route.

`version` on `purchase` doubles as the optimistic concurrency guard:

```sql
UPDATE purchase SET ..., version = version + 1
WHERE id = $1 AND user_id = $2 AND version = $3 AND deleted_at IS NULL
RETURNING *
```

0 rows returned → a follow-up existence check (`miss_reason`) decides: purchase gone or not owned → `RepositoryError::NotFound` (404); purchase exists → `RepositoryError::VersionConflict` (409).

---

## Analytics Projection

`purchase_fact` is an append-only TimescaleDB hypertable holding the reporting view
of a Purchase. `purchase` itself is deliberately **not** a hypertable: a hypertable's
unique indexes must include the partition key, which would break
`item.purchase_id -> purchase(id)`.

```
write path ──same tx──► analytics_outbox ──┐
                              (ids only)   │  pg_notify on commit
                                           ▼
                       FactProjector ── claim (FOR UPDATE SKIP LOCKED)
                                     ── rebuild_fact: DELETE + INSERT .. SELECT
                                     ── mark_processed
                                           │
                                           ▼
                                    purchase_fact (amounts in purchase currency
                                    and wallet currency + resolution status)
```

- **Events carry ids, never a payload or field diff.** The projector re-reads
  current OLTP state, so processing is idempotent, order-insensitive, and several
  edits to one Purchase collapse into a single rebuild.
- **Rebuild is delete-then-insert**, because an edited `purchase_date` changes the
  partition key. Applying it twice leaves identical state.
- **Waking.** `pg_notify` is transactional, so it fires on COMMIT only. A dedicated
  `LISTEN` connection wakes the projector; the 1 s poll remains as a mandatory
  fallback, because notifications are not queued for absent listeners. The listener
  cannot use the bb8 pool: bb8-postgres drives the `Connection` future itself and
  discards notification messages, which tokio-postgres only surfaces through
  `Connection::poll_message`.
- **Unresolved rates.** A Purchase with no conversion projects with
  `amount_base_minor IS NULL` and `resolution = 'unresolved'`, and is excluded from
  converted totals.
- **Maintenance.** `--backfill-facts` (enqueue and drain), `--reconcile-facts`
  (compare projected totals against a re-derivation from items, exiting non-zero on
  mismatch), `--report-stuck-outbox` (rows past the retry cap; exits non-zero) and
  `--purge-outbox` (delete processed rows older than `PURGE_KEEP_DAYS`). Rows that
  exhaust their retries stop being claimed and are excluded from the lag gauge, so
  they are surfaced explicitly rather than vanishing.
- **The projector depends on `AnalyticsRepository`, not on a concrete type**, so it
  can be exercised with a fake and replaced without touching application code.

---

## What to Build Next

1. Real exchange rate provider — swap `MockExchangeRateProvider` (it only serves pre-loaded pairs or same-currency 1.0)
2. Real key provider — replace `StaticKeyProvider` (empty map) with a client to the external key service owning key material + rotation
3. Expose purchase history — wire `PurchaseHistoryService`/`PgPurchaseHistoryRepo` into AppState and an API route
4. Expose the analytics endpoints (Phase 1) that read `purchase_fact`
5. Schedule the outbox purge; today `--purge-outbox` is a manual command
6. Run the integration tests in CI (no CI configuration exists yet)

---

## Observability

The metric and alert catalogue, the maintenance commands and the known gaps live in
`docs/OBSERVABILITY.md`; that file is the reference, this section is the summary.

OTLP traces, logs, and metrics are all wired (`config::Config::init` builds three layers on the tracing registry: `tracing_opentelemetry::layer` for traces, `OpenTelemetryTracingBridge` for logs with trace/span correlation, and `MetricsLayer` for metrics emitted from events).

`TraceLayer` on the router creates a custom span per request (`http.request.method/host/url`, `otel.name`, `http.response.status`) and its `on_response` hook emits:

- `histogram.http_request_duration_ms` — request latency
- `counter.http_5xx_total` — 5xx error counter (alert source)

Services use `#[instrument]` (already in place) and can emit business metrics as event fields:

```rust
use tracing::instrument;

impl<P, I, R, W, F> PurchaseService<P, I, R, W, F> {
    #[instrument(skip(self), fields(user_id = %cmd.user_id, wallet_id = %cmd.wallet_id))]
    pub async fn create(&self, cmd: CreatePurchaseCommand) -> Result<Purchase, ApplicationError> {
        // ...
        tracing::info!(monotonic_counter.purchase_created_total = 1);
    }
}
```

`#[instrument]` emits spans picked up by the `tracing-opentelemetry` layer, exported via OTLP. Metrics via `counter.`/`histogram.` field prefixes are picked up by `MetricsLayer` and exported via OTLP. Zero extra code beyond the event field.

---

## Technology Decisions

| Concern | Choice | Reason |
|---|---|---|
| Web framework | Axum 0.7 | Already in use. Tower ecosystem, good extractor model |
| DB driver | tokio-postgres + bb8 | Already in use. No ORM overhead, full SQL control |
| Migrations | refinery | Already in use. Standalone binary, version-controlled SQL (U1–U5) |
| Serialization | serde + serde_json | Standard |
| Error handling | thiserror | Already in use. Derive macros, no runtime cost |
| Auth | PASETO v4.local (`pasetors`) + `KeyProvider` trait | Footer `kid` → external key service; verifier swappable via `TokenVerifier` |
| Observability | OpenTelemetry + OTLP | Already in use. Vendor-neutral traces + logs + metrics |
| IDs | uuid v4 | Already in use. No coordination needed |
| Timestamps | chrono + TIMESTAMPTZ | Already in use. Always UTC in DB |
| Async runtime | tokio | Already in use |

Don't add `sqlx` — `tokio-postgres` works, repo traits abstract DB. Don't add `diesel` — same reason. No ORM.

Don't add `anyhow` — `thiserror` with explicit types better for layered arch needing cross-boundary error matching. `anyhow` for "just propagate" code, not three distinct error domains.
