#![allow(async_fn_in_trait)]

use thiserror::Error;

use crate::application::error::ApplicationError;
use crate::application::purchase::diff::PurchaseChanges;
use crate::model::{
    budget::{Budget, BudgetPeriod}, category::Category, income::{IncomeReceipt, IncomeSchedule},
    item::Item, ledger::{LedgerEntry, WalletBalance}, purchase::Purchase,
    purchase_history::PurchaseHistory, recurring::{RecurringRule, RecurringStatus},
    transfer::Transfer, user_profile::UserProfile, wallet::Wallet,
};
use crate::repository::ledger::NewLedgerEntry;

pub type Result<T> = std::result::Result<T, ApplicationError>;

// ---------------------------------------------------------------------------
// Wallet
// ---------------------------------------------------------------------------

pub trait WalletRepository {
    async fn create(&self, user_id: uuid::Uuid, name: String, currency: String) -> Result<Wallet>;
    async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Wallet>>;
    async fn get(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<Wallet>;
    async fn update(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
        name: String,
        currency: Option<String>,
    ) -> Result<Wallet>;
    async fn delete(&self, id: uuid::Uuid, user_id: uuid::Uuid) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Ledger
// ---------------------------------------------------------------------------

/// Append-only record of money movements. A Wallet's balance is the sum of its
/// entries (ADR-0004), so this trait has no update or delete: a correction is
/// a new entry that offsets the old one.
pub trait LedgerRepository {
    /// Appends one entry.
    async fn append(&self, entry: NewLedgerEntry) -> Result<LedgerEntry>;
    /// A Wallet's balance, in the Wallet's own currency.
    async fn balance(&self, user_id: uuid::Uuid, wallet_id: uuid::Uuid) -> Result<WalletBalance>;
    /// A Wallet's entries, newest first.
    async fn list(
        &self,
        user_id: uuid::Uuid,
        wallet_id: uuid::Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LedgerEntry>>;
}

// ---------------------------------------------------------------------------
// Transfer
// ---------------------------------------------------------------------------

/// A transfer between two of the user's own Wallets.
///
/// The repository writes both ledger legs in one transaction: they must net to
/// zero, and a partial write would silently create or destroy money in
/// aggregate figures (ADR-0004).
pub trait TransferRepository {
    /// Records a transfer and its two ledger legs.
    async fn create(&self, transfer: Transfer) -> Result<Transfer>;
    async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Transfer>>;
    async fn get(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<Transfer>;
    /// Deletes a transfer and reverses its legs.
    async fn delete(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Income
// ---------------------------------------------------------------------------

/// An income schedule and its occurrences.
///
/// A schedule is an *expectation*: it becomes money only when an occurrence is
/// received (ADR-0008). The repository writes the ledger credits in the same
/// transaction as the receipt, so a recorded occurrence and the money it
/// represents cannot disagree.
pub trait IncomeRepository {
    async fn create(&self, schedule: IncomeSchedule) -> Result<IncomeSchedule>;
    async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<IncomeSchedule>>;
    async fn get(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<IncomeSchedule>;
    /// Records an occurrence and credits the wallets it was directed to.
    async fn receive(
        &self,
        user_id: uuid::Uuid,
        schedule_id: uuid::Uuid,
        occurred_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<IncomeReceipt>;
    async fn delete(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Recurring rule
// ---------------------------------------------------------------------------

/// A declared recurring commitment.
///
/// The burn rate excludes the recorded payments of these rules, so that the
/// forecast does not subtract a subscription twice (once inside the trailing
/// spend, once as a dated movement).
pub trait RecurringRuleRepository {
    async fn create(&self, rule: RecurringRule) -> Result<RecurringRule>;
    async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<RecurringRule>>;
    async fn get(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<RecurringRule>;
    /// Sets a rule's status. A cancelled rule stops being forecast.
    async fn set_status(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
        status: RecurringStatus,
    ) -> Result<RecurringRule>;
    async fn delete(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// A budget template and its materialised periods.
///
/// Periods are materialised lazily, when a period is first asked about
/// (ADR-0009). The unique constraint on (budget_id, period_start) keeps two
/// concurrent requests from creating the same one twice.
pub trait BudgetRepository {
    async fn create(&self, budget: Budget) -> Result<Budget>;
    async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Budget>>;
    async fn get(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<Budget>;
    /// The period starting on `period_start`, if it has been materialised.
    async fn find_period(
        &self,
        user_id: uuid::Uuid,
        budget_id: uuid::Uuid,
        period_start: chrono::NaiveDate,
    ) -> Result<Option<BudgetPeriod>>;
    /// Materialises a period. Idempotent on (budget_id, period_start).
    async fn insert_period(&self, period: BudgetPeriod) -> Result<BudgetPeriod>;
    /// Sets a period's limit. An edit applies to that period only (ADR-0011).
    async fn set_period_limit(
        &self,
        user_id: uuid::Uuid,
        budget_id: uuid::Uuid,
        period_start: chrono::NaiveDate,
        amount_minor: i64,
    ) -> Result<BudgetPeriod>;
    async fn delete(&self, user_id: uuid::Uuid, id: uuid::Uuid) -> Result<()>;
}

// ---------------------------------------------------------------------------
// User profile
// ---------------------------------------------------------------------------

/// The caller's *intent*: each field left as `None` keeps its current value.
///
/// Bundled as one type because these three always travel together and, as three
/// same-typed positionals, they already caused a real argument-order defect once
/// in this codebase.
#[derive(Debug, Clone, Default)]
pub struct UserProfileUpdate {
    pub timezone: Option<String>,
    pub base_currency: Option<String>,
    pub locale: Option<String>,
}

/// A fully resolved profile, validated and ready to store.
#[derive(Debug, Clone)]
pub struct UserProfileValues {
    pub timezone: String,
    pub base_currency: String,
    pub locale: Option<String>,
}

pub trait UserProfileRepository {
    /// `None` when the user has never saved a profile.
    async fn get(&self, user_id: uuid::Uuid) -> Result<Option<UserProfile>>;
    async fn upsert(
        &self,
        user_id: uuid::Uuid,
        values: UserProfileValues,
    ) -> Result<UserProfile>;
}

// ---------------------------------------------------------------------------
// Purchase
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct PurchaseFilters {
    pub merchant_id: Option<uuid::Uuid>,
    pub wallet_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    pub store: Option<String>,
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// A PurchaseFilters whose defaults have been filled in and whose bounds are
/// clamped. Unlike the input type, nothing here is optional any more: consumers
/// read the fields directly instead of guessing or panicking.
#[derive(Debug, Clone)]
pub struct NormalizedFilters {
    pub merchant_id: Option<uuid::Uuid>,
    pub wallet_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    pub store: Option<String>,
    pub start_date: chrono::DateTime<chrono::Utc>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: i64,
    pub offset: i64,
}

/// Default page size when a caller does not ask for one.
pub const DEFAULT_PAGE_LIMIT: i64 = 50;
/// Upper bound on a page, so an oversized request cannot ask for everything.
pub const MAX_PAGE_LIMIT: i64 = 200;
/// How far back a listing looks when no range is given.
pub const DEFAULT_WINDOW_DAYS: i64 = 30;

impl From<PurchaseFilters> for NormalizedFilters {
    /// Fills in defaults and clamps the caller's values.
    ///
    /// The API, service and repository layers all apply the same policy through
    /// this one conversion rather than each restating it. Applying it twice is
    /// not possible by construction: the input type carries options, the output
    /// type does not.
    fn from(filters: PurchaseFilters) -> Self {
        Self {
            merchant_id: filters.merchant_id,
            wallet_id: filters.wallet_id,
            category_id: filters.category_id,
            store: filters.store,
            start_date: filters.start_date.unwrap_or_else(|| {
                chrono::Utc::now() - chrono::Duration::days(DEFAULT_WINDOW_DAYS)
            }),
            end_date: filters.end_date,
            limit: filters
                .limit
                .unwrap_or(DEFAULT_PAGE_LIMIT)
                .clamp(1, MAX_PAGE_LIMIT),
            offset: filters.offset.unwrap_or(0).max(0),
        }
    }
}

/// The outcome of resolving a conversion for a Purchase.
#[derive(Debug, Clone)]
pub enum RateOutcome {
    /// A conversion was found.
    Resolved(RateUpdate),
    /// Recording went ahead without a conversion. `attempted` distinguishes
    /// "no conversion needed" from "tried and failed" (ADR-0006).
    Unresolved { attempted: bool },
}

/// A conversion the system or the user has established for a Purchase. Bundled
/// as one type because the three fields only make sense together and would
/// otherwise be three same-typed optionals in a row.
#[derive(Clone, Debug)]
pub struct RateUpdate {
    pub rate_scaled: i64,
    pub source: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct PurchaseUpdates {
    pub merchant_id: Option<uuid::Uuid>,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub category_id: Option<i32>,
    /// Correcting when the Purchase happened. A change here moves the row between
    /// projection partitions, which the rebuild's delete-then-insert handles.
    pub purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    /// `None` leaves the stored conversion untouched.
    pub rate: Option<RateUpdate>,
}

pub trait PurchaseRepository {
    async fn create(&self, params: CreatePurchaseParams) -> Result<Purchase>;
    async fn create_with_items(
        &self,
        params: CreatePurchaseParams,
        items: Vec<ItemInput>,
    ) -> Result<(Purchase, Vec<Item>)>;
    async fn get(&self, id: uuid::Uuid, user_id: uuid::Uuid) -> Result<Purchase>;
    async fn list(
        &self,
        user_id: uuid::Uuid,
        filters: NormalizedFilters,
    ) -> Result<Vec<Purchase>>;
    /// Total rows matching the filters, ignoring limit/offset.
    async fn count(&self, user_id: uuid::Uuid, filters: NormalizedFilters) -> Result<i64>;
    async fn update(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        purchase_update: PurchaseUpdates,
        changes: Option<PurchaseChanges>,
    ) -> Result<Purchase>;
    async fn delete(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
        changes: PurchaseChanges,
    ) -> Result<()>;
    async fn add_items(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        items: Vec<ItemInput>,
    ) -> Result<(Purchase, Vec<Item>)>;
    async fn update_item(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        sequence: i32,
        item: ItemUpdate,
        changes: PurchaseChanges,
    ) -> Result<Purchase>;
    async fn remove_item(
        &self,
        purchase_id: uuid::Uuid,
        user_id: uuid::Uuid,
        current_version: i32,
        sequence: i32,
        changes: PurchaseChanges,
    ) -> Result<Purchase>;
}

/// Parameters required to create a purchase.
#[derive(Clone, Debug)]
pub struct CreatePurchaseParams {
    pub wallet_id: uuid::Uuid,
    pub store: Option<String>,
    pub notes: Option<String>,
    pub user_id: uuid::Uuid,
    pub category_id: Option<i32>,
    pub merchant_id: Option<uuid::Uuid>,
    pub description: Option<String>,
    pub purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    pub currency: String,
    pub exchange_rate_scaled: Option<i64>,
    pub rate_source: Option<String>,
    pub rate_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Set by the application when the conversion failed and must be completed
    /// later (ADR-0006). The repository records the job; it does not decide policy.
    pub requires_rate_completion: bool,
}

// ---------------------------------------------------------------------------
// Item
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ItemInput {
    pub name: String,
    pub quantity: i32,
    pub unit_price: i64,
    pub item_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ItemUpdate {
    pub name: Option<String>,
    pub unit_price: Option<i64>,
    pub quantity: Option<i32>,
    pub item_type: Option<String>,
}

pub trait ItemRepository {
    async fn create(&self, purchase_id: uuid::Uuid, item: ItemInput) -> Result<Item>;
    async fn bulk_create(
        &self,
        purchase_id: uuid::Uuid,
        items: Vec<ItemInput>,
    ) -> Result<Vec<Item>>;
    async fn get(&self, purchase_id: uuid::Uuid, sequence: i32) -> Result<Item>;
    async fn update(
        &self,
        purchase_id: uuid::Uuid,
        sequence: i32,
        item: ItemUpdate,
    ) -> Result<Item>;
    async fn delete(&self, purchase_id: uuid::Uuid, sequence: i32) -> Result<()>;
    async fn list_by_purchase(&self, purchase_id: uuid::Uuid) -> Result<Vec<Item>>;
}

// ---------------------------------------------------------------------------
// Category
// ---------------------------------------------------------------------------

pub trait CategoryRepository {
    async fn create(
        &self,
        name: String,
        description: Option<String>,
        user_id: Option<uuid::Uuid>,
        is_system: bool,
    ) -> Result<Category>;
    async fn list(&self, user_id: Option<uuid::Uuid>) -> Result<Vec<Category>>;
    async fn get(&self, id: i32) -> Result<Category>;
}

// ---------------------------------------------------------------------------
// PurchaseHistory
// ---------------------------------------------------------------------------

pub trait PurchaseHistoryRepository {
    async fn get_for_purchase(&self, purchase_id: uuid::Uuid) -> Result<Vec<PurchaseHistory>>;
}

// ---------------------------------------------------------------------------
// Exchange rate cache (top-of-hour, unlogged table)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CachedRate {
    pub from_currency: String,
    pub to_currency: String,
    pub rate_hour: chrono::DateTime<chrono::Utc>,
    pub rate_scaled: i64,
    pub source: String,
    pub stale: bool,
}

/// Durable rate history, written on every successful fetch. The unlogged
/// exchange_rate_cache is re-fetchable; this is not.
/// Reads the durable rate history: the fallback when the unlogged cache has been
/// emptied by a restart.
pub trait RateHistoryReader {
    /// Most recent recorded rate for a pair, or `None` when nothing was recorded.
    async fn latest_history(
        &self,
        from_currency: &str,
        to_currency: &str,
    ) -> Result<Option<CachedRate>>;
}

pub trait RateHistoryWriter {
    async fn record_history(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
        rate_scaled: i64,
        source: &str,
    ) -> Result<()>;
}

pub trait RateCacheRepository {
    async fn get(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<CachedRate>>;
    async fn get_most_recent(
        &self,
        from_currency: &str,
        to_currency: &str,
    ) -> Result<Option<CachedRate>>;
    async fn put(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
        rate_scaled: i64,
        source: &str,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Analytics projection
// ---------------------------------------------------------------------------

/// Attempts after which an outbox row is considered exhausted and is no longer
/// claimed. Public so the maintenance interface and its tests share one value.
pub const MAX_ATTEMPTS: i32 = 10;

/// One claimed outbox row awaiting projection.
#[derive(Debug, Clone)]
pub struct PendingFact {
    pub outbox_id: i64,
    pub user_id: uuid::Uuid,
    pub purchase_id: uuid::Uuid,
}

/// An outbox row that has exhausted its retries and is no longer claimed.
#[derive(Debug, Clone)]
pub struct StuckOutboxItem {
    pub outbox_id: i64,
    pub user_id: uuid::Uuid,
    pub purchase_id: uuid::Uuid,
    pub attempts: i32,
    pub last_error: Option<String>,
}

/// A `(user, base_currency, day)` bucket where the projection disagrees with OLTP.
#[derive(Debug, Clone)]
pub struct ReconcileMismatch {
    pub user_id: uuid::Uuid,
    pub base_currency: String,
    pub day: chrono::NaiveDate,
    pub fact_total: i64,
    pub oltp_total: i64,
}

/// Storage contract for the analytics projection.
///
/// The application layer depends on this interface, never on a concrete
/// implementation, so the projector and maintenance operations can be driven by
/// a fake in tests and replaced without touching application code.
/// A Purchase recorded without a conversion, awaiting completion.
#[derive(Debug, Clone)]
pub struct UnresolvedPurchase {
    pub purchase_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub source_currency: String,
    pub wallet_currency: String,
    pub amount_minor: i64,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

/// A queued job awaiting a worker.
#[derive(Debug, Clone)]
pub struct QueuedJob {
    pub id: i64,
    pub user_id: uuid::Uuid,
    pub payload: serde_json::Value,
}

/// A recorded purchase, as the burn rate needs to see it.
#[derive(Debug, Clone)]
pub struct SpendPurchase {
    pub purchase_id: uuid::Uuid,
    pub merchant_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    /// The purchase's own amount, in its purchase currency.
    pub amount_minor: i64,
    pub currency: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

/// Failures while completing an unresolved rate.
#[derive(Debug, Error)]
pub enum CompletionError {
    #[error(transparent)]
    Rate(#[from] crate::application::outbound::exchange_rate::RateError),
    #[error(transparent)]
    Application(#[from] crate::application::error::ApplicationError),
}

pub trait JobQueueRepository {
    /// Claims the oldest due job of `kind`, or `None` when none is due.
    async fn claim_due_job(&self, kind: &str) -> Result<Option<QueuedJob>>;
    async fn complete_job(&self, id: i64) -> Result<()>;
    /// Records a failed attempt and schedules a retry with backoff. `failed`
    /// marks the job exhausted (no further retries).
    async fn retry_job(&self, id: i64, error: &str) -> Result<u32>;
    /// Enqueues a completion job for every purchase recorded without a
    /// conversion. Idempotent-ish: jobs for purchases that already have a rate
    /// complete as no-ops. Returns how many were queued.
    async fn enqueue_resolve_jobs(&self) -> Result<u64>;
}

pub trait AnalyticsRepository {
    /// Total + count series in the user's timezone, grouped per currency.
    async fn bucketed_spend(
        &self,
        user_id: uuid::Uuid,
        range: &SpendRange,
        bucket: TrendBucket,
    ) -> Result<Vec<CurrencySeries>>;
    /// Daily totals with a trailing-window average, per currency.
    async fn daily_spend(
        &self,
        user_id: uuid::Uuid,
        range: &SpendRange,
        window_days: i32,
    ) -> Result<Vec<DailySeries>>;
    /// Atomically claims up to `limit` pending outbox rows.
    async fn claim_outbox(&self, limit: i64) -> Result<Vec<PendingFact>>;
    async fn mark_processed(&self, ids: &[i64]) -> Result<()>;
    async fn mark_failed(&self, id: i64, error: &str) -> Result<()>;
    /// Deletes and re-inserts the projected row for one Purchase.
    async fn rebuild_fact(&self, user_id: uuid::Uuid, purchase_id: uuid::Uuid) -> Result<()>;
    /// Queues every live Purchase for projection.
    async fn enqueue_all_purchases(&self) -> Result<u64>;
    async fn purge_processed_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64>;
    async fn stuck_outbox(&self) -> Result<Vec<StuckOutboxItem>>;
    /// Purchases recorded with an unresolved conversion (story 16).
    async fn unresolved_purchases(&self) -> Result<Vec<UnresolvedPurchase>>;
    /// Age of the oldest pending row in seconds; `None` when nothing is pending.
    async fn outbox_lag_seconds(&self) -> Result<Option<f64>>;
    /// The recorded purchases in a range, so the burn rate can tell which of
    /// them are the payments of a declared recurring rule.
    async fn purchases_in_range(
        &self,
        user_id: uuid::Uuid,
        range: &SpendRange,
    ) -> Result<Vec<SpendPurchase>>;
    /// The earliest date the user has recorded a purchase.
    ///
    /// The burn rate measures over the history the user actually has, which
    /// starts at their first purchase.
    async fn first_purchase_date(
        &self,
        user_id: uuid::Uuid,
        wallet_id: Option<uuid::Uuid>,
    ) -> Result<Option<chrono::NaiveDate>>;
    async fn reconcile(&self) -> Result<Vec<ReconcileMismatch>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filters(limit: Option<i64>, offset: Option<i64>) -> PurchaseFilters {
        PurchaseFilters {
            limit,
            offset,
            ..Default::default()
        }
    }

    #[test]
    fn normalization_fills_defaults_and_clamps() {
        let default: NormalizedFilters = filters(None, None).into();
        assert_eq!(default.limit, DEFAULT_PAGE_LIMIT);
        assert_eq!(default.offset, 0);
        assert!(default.start_date <= chrono::Utc::now(), "a listing always has a window");

        let clamped: NormalizedFilters = filters(Some(9999), Some(-5)).into();
        assert_eq!(clamped.limit, MAX_PAGE_LIMIT);
        assert_eq!(clamped.offset, 0);

        let tiny: NormalizedFilters = filters(Some(0), None).into();
        assert_eq!(tiny.limit, 1, "a page is at least one row");
    }
}

// ---------------------------------------------------------------------------
// Trends (read model over the projection)
// ---------------------------------------------------------------------------

/// Bucket width for a trend series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrendBucket {
    Day,
    Week,
    Month,
}

impl TrendBucket {
    pub fn as_interval(self) -> &'static str {
        match self {
            TrendBucket::Day => "1 day",
            TrendBucket::Week => "1 week",
            TrendBucket::Month => "1 month",
        }
    }
}

/// A read range plus the scoping filters a series can carry. Bundled because the
/// fields travel together in every trends query.
#[derive(Debug, Clone)]
pub struct SpendRange {
    pub from: chrono::DateTime<chrono::Utc>,
    pub to: chrono::DateTime<chrono::Utc>,
    pub wallet_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    /// Restrict to one currency (series are already per-currency; this selects one).
    pub currency: Option<String>,
    /// Bucket days in the user's calendar (query-time bucketing, D25).
    pub timezone: String,
}

/// One bucket of spend in one currency. Totals are per-currency minor units
/// (ADR-0002: never summed across currencies); `unresolved` counts Purchases in
/// the bucket with no conversion yet, which contribute nothing to the totals.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeriesPoint {
    pub bucket_start: chrono::DateTime<chrono::Utc>,
    pub total_minor: i64,
    pub purchases: i64,
    pub unresolved: i64,
}

/// A trend series for one currency.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CurrencySeries {
    pub currency: String,
    pub points: Vec<SeriesPoint>,
}

/// One day of spend in one currency.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyPoint {
    pub day: chrono::NaiveDate,
    pub total_minor: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DailySeries {
    pub currency: String,
    pub points: Vec<DailyPoint>,
}
