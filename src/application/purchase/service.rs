use chrono::Timelike;
use chrono::{DateTime, Utc};
use tracing::{instrument, warn};

use crate::application::{
    error::ApplicationError,
    inbound::repositories::{
        CreatePurchaseParams, ItemInput, ItemRepository, ItemUpdate, PurchaseFilters,
        PurchaseRepository, PurchaseUpdates, RateCacheRepository, WalletRepository,
    },
    outbound::exchange_rate::{ExchangeRateProvider, RateRequest},
};
use crate::model::item::Item;
use crate::model::purchase::Purchase;

use super::commands::{
    AddItemsCommand, CreatePurchaseCommand, RemoveItemCommand, UpdateItemCommand,
    UpdatePurchaseCommand,
};
use super::diff::{diff_fields, ItemDiff, ItemSnapshot, PurchaseChanges};


pub struct PurchaseService<P, I, R, W, F>
where
    P: PurchaseRepository,
    I: ItemRepository,
    R: RateCacheRepository,
    W: WalletRepository,
    F: ExchangeRateProvider,
{
    purchases: P,
    items: I,
    rate_cache: R,
    wallets: W,
    fx: F,
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
        Self {
            purchases,
            items,
            rate_cache,
            wallets,
            fx,
        }
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %cmd.user_id, wallet_id = %cmd.wallet_id, item_count = cmd.items.len()))]
    pub async fn create(&self, cmd: CreatePurchaseCommand) -> Result<Purchase, ApplicationError> {
        // 1. Input validation
        let currency = crate::application::currency::normalize_currency(&cmd.currency)?;
        validate_items(&cmd.items)?;

        // 2. Load wallet to determine its holding currency
        let wallet = self.wallets.get(cmd.user_id, cmd.wallet_id).await?;

        // 3. Rate resolution
        let (exchange_rate_scaled, rate_source, rate_at) = self
            .resolve_rate(&cmd, &currency, &wallet.currency, cmd.purchase_date)
            .await?;

        // 4. Atomic persist (purchase + items in one transaction)
        let params = CreatePurchaseParams {
            wallet_id: cmd.wallet_id,
            store: cmd.store,
            notes: cmd.notes,
            user_id: cmd.user_id,
            category_id: cmd.category_id,
            merchant_id: cmd.merchant_id,
            description: cmd.description,
            purchase_date: cmd.purchase_date,
            currency,
            exchange_rate_scaled,
            rate_source,
            rate_at,
        };
        let (purchase, _items) = self.purchases.create_with_items(params, cmd.items).await?;

        tracing::info!(
            monotonic_counter.purchase_created_total = 1,
            currency = %purchase.currency,
            rate_source = %purchase.rate_source.as_deref().unwrap_or("none"),
        );

        Ok(purchase)
    }

    async fn resolve_rate(
        &self,
        cmd: &CreatePurchaseCommand,
        source_currency: &str,
        wallet_currency: &str,
        purchase_date: Option<DateTime<Utc>>,
    ) -> Result<(Option<i64>, Option<String>, Option<DateTime<Utc>>), ApplicationError> {
        // Manual override wins: warn + flag, no cache/API.
        if let Some(override_rate) = cmd.exchange_rate_override {
            let rate_scaled = crate::application::currency::rate_to_scaled(override_rate)?;
            warn!(
                source_currency = %source_currency,
                wallet_currency = %wallet_currency,
                rate = %override_rate,
                "manual exchange rate override applied"
            );
            return Ok((
                Some(rate_scaled),
                Some("manual".to_string()),
                Some(Utc::now()),
            ));
        }

        // Same currency => no conversion needed.
        if source_currency == wallet_currency {
            return Ok((None, None, None));
        }

        // Top-of-hour cache lookup.
        let at = purchase_date.unwrap_or_else(Utc::now);
        let rate_hour = at
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .ok_or_else(|| {
                ApplicationError::Internal("failed to truncate rate timestamp to hour".into())
            })?;

        if let Some(cached) = self
            .rate_cache
            .get(source_currency, wallet_currency, rate_hour)
            .await?
        {
            tracing::info!(monotonic_counter.fx_rate_cache_total = 1u64, result = "hit");
            return Ok((
                Some(cached.rate_scaled),
                Some("api".to_string()),
                Some(cached.rate_hour),
            ));
        }

        // Cache miss => call provider once, populate cache.
        let req = RateRequest {
            from: source_currency.to_string(),
            to: wallet_currency.to_string(),
            at,
        };

        tracing::info!(
            monotonic_counter.fx_rate_cache_total = 1u64,
            result = "miss"
        );
        let fx_started = std::time::Instant::now();
        let fx_result = self.fx.fetch_rate(req).await;
        tracing::info!(
            histogram.fx_provider_duration_ms = fx_started.elapsed().as_millis() as f64,
            outcome = if fx_result.is_ok() { "ok" } else { "error" },
        );
        match fx_result {
            Ok(result) => {
                let rate_scaled = crate::application::currency::rate_to_scaled(result.rate)?;
                self.rate_cache
                    .put(
                        source_currency,
                        wallet_currency,
                        rate_hour,
                        rate_scaled,
                        &result.source,
                    )
                    .await?;
                Ok((Some(rate_scaled), Some(result.source), Some(rate_hour)))
            }
            Err(e) => {
                let kind = match &e {
                    crate::application::outbound::exchange_rate::RateError::ProviderUnavailable => {
                        "provider_unavailable"
                    }
                    crate::application::outbound::exchange_rate::RateError::PairNotFound {
                        ..
                    } => "pair_not_found",
                    crate::application::outbound::exchange_rate::RateError::InvalidRate {
                        ..
                    } => "invalid_rate",
                };
                tracing::info!(monotonic_counter.fx_provider_failure_total = 1u64, kind = kind);
                tracing::error!(
                    kind = kind,
                    error = %e,
                    source_currency = %source_currency,
                    wallet_currency = %wallet_currency,
                    "exchange rate provider failed"
                );
                // Provider failed: fall back to most recent cached rate of any age.
                if let Some(prev) = self
                    .rate_cache
                    .get_most_recent(source_currency, wallet_currency)
                    .await?
                {
                    warn!(
                        source_currency = %source_currency,
                        wallet_currency = %wallet_currency,
                        rate_scaled = %prev.rate_scaled,
                        rate_hour = %prev.rate_hour,
                        "exchange rate provider unavailable; using stale cached rate"
                    );
                    tracing::info!(monotonic_counter.fx_stale_fallback_total = 1u64);
                    Ok((
                        Some(prev.rate_scaled),
                        Some(prev.source),
                        Some(prev.rate_hour),
                    ))
                } else {
                    Err(ApplicationError::Internal(format!(
                        "exchange rate unavailable for {source_currency}/{wallet_currency}"
                    )))
                }
            }
        }
    }

    #[instrument(skip_all, err(level = "warn"), fields(purchase_id = %id, user_id = %user_id))]
    pub async fn get(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<Purchase, ApplicationError> {
        let purchase = self.purchases.get(id, user_id).await?;
        Ok(purchase)
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(
        &self,
        user_id: uuid::Uuid,
        filters: PurchaseFilters,
    ) -> Result<Vec<Purchase>, ApplicationError> {
        let mut f = filters;
        if f.limit.is_none() {
            f.limit = Some(50);
        }
        if f.offset.is_none() {
            f.offset = Some(0);
        }
        if f.start_date.is_none() {
            f.start_date = Some(chrono::Utc::now() - chrono::Duration::days(30));
        }

        self.purchases.list(user_id, f).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(purchase_id = %cmd.id, user_id = %cmd.user_id))]
    pub async fn update(&self, cmd: UpdatePurchaseCommand) -> Result<Purchase, ApplicationError> {
        let before = self.purchases.get(cmd.id, cmd.user_id).await?;

        let updates = PurchaseUpdates {
            store: cmd.store,
            description: cmd.description,
            notes: cmd.notes,
            category_id: cmd.category_id,
            merchant_id: cmd.merchant_id,
        };

        let fields = diff_fields(&before, &updates);
        if fields.is_empty() {
            return Ok(before);
        }

        let changes = PurchaseChanges::PurchaseUpdated { fields };

        self.purchases
            .update(
                cmd.id,
                cmd.user_id,
                cmd.current_version,
                updates,
                Some(changes),
            )
            .await
    }

    #[instrument(skip_all, err(level = "warn"), fields(purchase_id = %cmd.id, user_id = %cmd.user_id))]
    pub async fn add_items(
        &self,
        cmd: AddItemsCommand,
    ) -> Result<(Purchase, Vec<crate::model::item::Item>), ApplicationError> {
        // Load purchase to enforce ownership + optimistic-lock version.
        let _purchase = self.purchases.get(cmd.id, cmd.user_id).await?;
        validate_items(&cmd.items)?;

        self.purchases
            .add_items(cmd.id, cmd.user_id, cmd.current_version, cmd.items)
            .await
    }

    #[instrument(skip_all, err(level = "warn"), fields(purchase_id = %cmd.id, item_sequence = %cmd.sequence, user_id = %cmd.user_id))]
    pub async fn update_item(&self, cmd: UpdateItemCommand) -> Result<Purchase, ApplicationError> {
        let purchase = self.purchases.get(cmd.id, cmd.user_id).await?;
        let before = self.items.get(cmd.id, cmd.sequence).await?;

        let updates = ItemUpdate {
            name: cmd.name,
            unit_price: cmd.unit_price,
            quantity: cmd.quantity,
            item_type: cmd.item_type,
        };

        // Project the resulting item so the diff can be built without mutating state.
        let after = Item {
            item_sequence: before.item_sequence,
            purchase_id: before.purchase_id,
            name: updates.name.clone().unwrap_or_else(|| before.name.clone()),
            unit_price: updates.unit_price.unwrap_or(before.unit_price),
            quantity: updates.quantity.unwrap_or(before.quantity),
            item_type: updates
                .item_type
                .clone()
                .or_else(|| before.item_type.clone()),
        };

        // Validate the resulting values so invalid inputs (e.g. quantity <= 0)
        // fail cleanly instead of surfacing as a DB error.
        let projected = ItemInput {
            name: after.name.clone(),
            quantity: after.quantity,
            unit_price: after.unit_price,
            item_type: after.item_type.clone(),
        };
        validate_items(&[projected])?;

        let diff = ItemDiff::new(&before, &after);
        if diff.from.name == diff.to.name
            && diff.from.unit_price == diff.to.unit_price
            && diff.from.quantity == diff.to.quantity
            && diff.from.item_type == diff.to.item_type
        {
            return Ok(purchase);
        }

        let changes = PurchaseChanges::ItemUpdated { items: vec![diff] };

        self.purchases
            .update_item(
                cmd.id,
                cmd.user_id,
                cmd.current_version,
                cmd.sequence,
                updates,
                changes,
            )
            .await
    }

    #[instrument(skip_all, err(level = "warn"), fields(purchase_id = %cmd.id, item_sequence = %cmd.sequence, user_id = %cmd.user_id))]
    pub async fn remove_item(&self, cmd: RemoveItemCommand) -> Result<Purchase, ApplicationError> {
        let _purchase = self.purchases.get(cmd.id, cmd.user_id).await?;
        let before = self.items.get(cmd.id, cmd.sequence).await?;

        let changes = PurchaseChanges::ItemRemoved {
            items: vec![ItemSnapshot::from(&before)],
        };

        self.purchases
            .remove_item(
                cmd.id,
                cmd.user_id,
                cmd.current_version,
                cmd.sequence,
                changes,
            )
            .await
    }

    #[instrument(skip_all, err(level = "warn"), fields(purchase_id = %id, user_id = %user_id))]
    pub async fn delete(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        let changes = PurchaseChanges::PurchaseDeleted;
        self.purchases.delete(id, user_id, changes).await
    }
}

fn validate_items(
    items: &[crate::application::inbound::repositories::ItemInput],
) -> Result<(), ApplicationError> {
    for item in items {
        if item.quantity <= 0 {
            return Err(ApplicationError::Validation(
                "item quantity must be greater than zero".into(),
            ));
        }
        if item.unit_price < 0 {
            return Err(ApplicationError::Validation(
                "item unit_price must be non-negative".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::inbound::repositories::Result as RepoResult;
    use crate::application::inbound::repositories::{
        CachedRate, CreatePurchaseParams, ItemInput, ItemRepository, PurchaseFilters,
        PurchaseRepository, PurchaseUpdates, RateCacheRepository, WalletRepository,
    };
    use crate::model::item::Item;
    use crate::model::wallet::Wallet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn sample_purchase(version: i32) -> Purchase {
        Purchase {
            id: uuid::Uuid::new_v4(),
            user_id: uuid::Uuid::new_v4(),
            wallet_id: uuid::Uuid::new_v4(),
            category_id: Some(1),
            classification_status: "confirmed".to_string(),
            merchant_id: None,
            store: Some("Store A".to_string()),
            description: Some("desc".to_string()),
            notes: None,
            version,
            currency: "USD".to_string(),
            exchange_rate_scaled: None,
            rate_source: None,
            rate_at: None,
            purchase_date: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        }
    }

    fn sample_item(seq: i32) -> Item {
        Item {
            item_sequence: seq,
            purchase_id: uuid::Uuid::new_v4(),
            name: "Milk".to_string(),
            unit_price: 100,
            quantity: 2,
            item_type: Some("grocery".to_string()),
        }
    }

    fn input(name: &str, qty: i32, price: i64) -> ItemInput {
        ItemInput {
            name: name.to_string(),
            quantity: qty,
            unit_price: price,
            item_type: None,
        }
    }

    // --- Mock repositories -------------------------------------------------

    struct MockPurchaseRepo {
        get_result: Option<Purchase>,
        update_calls: AtomicUsize,
        add_items_calls: AtomicUsize,
        update_item_calls: AtomicUsize,
        remove_item_calls: AtomicUsize,
        delete_calls: AtomicUsize,
        received_changes: Arc<std::sync::Mutex<Vec<PurchaseChanges>>>,
    }

    impl MockPurchaseRepo {
        fn new(get_result: Purchase) -> Self {
            Self {
                get_result: Some(get_result),
                update_calls: AtomicUsize::new(0),
                add_items_calls: AtomicUsize::new(0),
                update_item_calls: AtomicUsize::new(0),
                remove_item_calls: AtomicUsize::new(0),
                delete_calls: AtomicUsize::new(0),
                received_changes: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        fn changes(&self) -> Vec<PurchaseChanges> {
            self.received_changes.lock().unwrap().clone()
        }
    }

    impl PurchaseRepository for MockPurchaseRepo {
        async fn create(&self, _params: CreatePurchaseParams) -> RepoResult<Purchase> {
            todo!()
        }
        async fn create_with_items(
            &self,
            _params: CreatePurchaseParams,
            _items: Vec<ItemInput>,
        ) -> RepoResult<(Purchase, Vec<Item>)> {
            todo!()
        }
        async fn get(&self, _id: uuid::Uuid, _user_id: uuid::Uuid) -> RepoResult<Purchase> {
            self.get_result
                .clone()
                .ok_or(ApplicationError::NotFound("purchase".into()))
        }
        async fn list(
            &self,
            _user_id: uuid::Uuid,
            _filters: PurchaseFilters,
        ) -> RepoResult<Vec<Purchase>> {
            Ok(vec![])
        }
        async fn update(
            &self,
            _purchase_id: uuid::Uuid,
            _user_id: uuid::Uuid,
            _current_version: i32,
            _purchase_update: PurchaseUpdates,
            changes: Option<PurchaseChanges>,
        ) -> RepoResult<Purchase> {
            self.update_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(c) = changes {
                self.received_changes.lock().unwrap().push(c);
            }
            let mut p = self.get_result.clone().unwrap();
            p.version += 1;
            Ok(p)
        }
        async fn delete(
            &self,
            _id: uuid::Uuid,
            _user_id: uuid::Uuid,
            changes: PurchaseChanges,
        ) -> RepoResult<()> {
            self.delete_calls.fetch_add(1, Ordering::SeqCst);
            self.received_changes.lock().unwrap().push(changes);
            Ok(())
        }
        async fn add_items(
            &self,
            _purchase_id: uuid::Uuid,
            _user_id: uuid::Uuid,
            _current_version: i32,
            _items: Vec<ItemInput>,
        ) -> RepoResult<(Purchase, Vec<Item>)> {
            self.add_items_calls.fetch_add(1, Ordering::SeqCst);
            Ok((self.get_result.clone().unwrap(), vec![]))
        }
        async fn update_item(
            &self,
            _purchase_id: uuid::Uuid,
            _user_id: uuid::Uuid,
            _current_version: i32,
            _sequence: i32,
            _item: ItemUpdate,
            changes: PurchaseChanges,
        ) -> RepoResult<Purchase> {
            self.update_item_calls.fetch_add(1, Ordering::SeqCst);
            self.received_changes.lock().unwrap().push(changes);
            let mut p = self.get_result.clone().unwrap();
            p.version += 1;
            Ok(p)
        }
        async fn remove_item(
            &self,
            _purchase_id: uuid::Uuid,
            _user_id: uuid::Uuid,
            _current_version: i32,
            _sequence: i32,
            changes: PurchaseChanges,
        ) -> RepoResult<Purchase> {
            self.remove_item_calls.fetch_add(1, Ordering::SeqCst);
            self.received_changes.lock().unwrap().push(changes);
            let mut p = self.get_result.clone().unwrap();
            p.version += 1;
            Ok(p)
        }
    }

    struct MockItemRepo {
        get_result: Item,
    }

    impl MockItemRepo {
        fn new(get_result: Item) -> Self {
            Self { get_result }
        }
    }

    impl ItemRepository for MockItemRepo {
        async fn create(&self, _purchase_id: uuid::Uuid, _item: ItemInput) -> RepoResult<Item> {
            todo!()
        }
        async fn bulk_create(
            &self,
            _purchase_id: uuid::Uuid,
            _items: Vec<ItemInput>,
        ) -> RepoResult<Vec<Item>> {
            todo!()
        }
        async fn get(&self, _purchase_id: uuid::Uuid, _sequence: i32) -> RepoResult<Item> {
            Ok(self.get_result.clone())
        }
        async fn update(
            &self,
            _purchase_id: uuid::Uuid,
            _sequence: i32,
            _item: ItemUpdate,
        ) -> RepoResult<Item> {
            todo!()
        }
        async fn delete(&self, _purchase_id: uuid::Uuid, _sequence: i32) -> RepoResult<()> {
            todo!()
        }
        async fn list_by_purchase(&self, _purchase_id: uuid::Uuid) -> RepoResult<Vec<Item>> {
            todo!()
        }
    }

    // Stub repos for the unused generics (RateCache, Wallet, FX, History, Category).
    struct MockRateCache;
    impl RateCacheRepository for MockRateCache {
        async fn get(
            &self,
            _a: &str,
            _b: &str,
            _h: chrono::DateTime<chrono::Utc>,
        ) -> RepoResult<Option<CachedRate>> {
            Ok(None)
        }
        async fn get_most_recent(&self, _a: &str, _b: &str) -> RepoResult<Option<CachedRate>> {
            Ok(None)
        }
        async fn put(
            &self,
            _a: &str,
            _b: &str,
            _h: chrono::DateTime<chrono::Utc>,
            _r: i64,
            _s: &str,
        ) -> RepoResult<()> {
            Ok(())
        }
    }

    struct MockWallet;
    impl WalletRepository for MockWallet {
        async fn create(
            &self,
            _u: uuid::Uuid,
            _n: String,
            _c: String,
        ) -> RepoResult<Wallet> {
            todo!()
        }
        async fn list(&self, _u: uuid::Uuid) -> RepoResult<Vec<Wallet>> {
            todo!()
        }
        async fn get(&self, _u: uuid::Uuid, _id: uuid::Uuid) -> RepoResult<Wallet> {
            todo!()
        }
        async fn update(
            &self,
            _i: uuid::Uuid,
            _u: uuid::Uuid,
            _n: String,
            _c: Option<String>,
        ) -> RepoResult<Wallet> {
            todo!()
        }
        async fn delete(&self, _i: uuid::Uuid, _u: uuid::Uuid) -> RepoResult<()> {
            todo!()
        }
    }

    struct MockFx;
    impl ExchangeRateProvider for MockFx {
        async fn fetch_rate(
            &self,
            _req: crate::application::outbound::exchange_rate::RateRequest,
        ) -> Result<
            crate::application::outbound::exchange_rate::RateResult,
            crate::application::outbound::exchange_rate::RateError,
        > {
            todo!()
        }
    }

    // --- Helpers -----------------------------------------------------------

    fn service(
        purchases: MockPurchaseRepo,
        items: MockItemRepo,
    ) -> PurchaseService<MockPurchaseRepo, MockItemRepo, MockRateCache, MockWallet, MockFx> {
        PurchaseService::new(purchases, items, MockRateCache, MockWallet, MockFx)
    }

    // --- update ------------------------------------------------------------

    #[tokio::test]
    async fn update_with_no_field_changes_skips_repo_call() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = UpdatePurchaseCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            store: Some("Store A".to_string()), // same as current
            description: None,
            notes: None,
            category_id: None,
            merchant_id: None,
        };
        let result = svc.update(cmd).await.unwrap();
        assert_eq!(result.version, 5);
        assert_eq!(svc.purchases.update_calls.load(Ordering::SeqCst), 0);
        assert!(svc.purchases.changes().is_empty());
    }

    #[tokio::test]
    async fn update_with_changed_field_forwards_purchase_updated_changes() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = UpdatePurchaseCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            store: Some("Store B".to_string()),
            description: None,
            notes: None,
            category_id: None,
            merchant_id: None,
        };
        let result = svc.update(cmd).await.unwrap();
        assert_eq!(result.version, 6);
        assert_eq!(svc.purchases.update_calls.load(Ordering::SeqCst), 1);
        let changes = svc.purchases.changes();
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            PurchaseChanges::PurchaseUpdated { fields } => {
                let store = fields.store.as_ref().unwrap();
                assert_eq!(store.from, "Store A");
                assert_eq!(store.to, "Store B");
            }
            _ => panic!("expected purchase_updated"),
        }
    }

    #[tokio::test]
    async fn update_with_empty_command_forwards_nothing() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = UpdatePurchaseCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            store: None,
            description: None,
            notes: None,
            category_id: None,
            merchant_id: None,
        };
        svc.update(cmd).await.unwrap();
        assert_eq!(svc.purchases.update_calls.load(Ordering::SeqCst), 0);
    }

    // --- delete ------------------------------------------------------------

    #[tokio::test]
    async fn delete_records_purchase_deleted() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        svc.delete(before.id, before.user_id).await.unwrap();
        assert_eq!(svc.purchases.delete_calls.load(Ordering::SeqCst), 1);
        let changes = svc.purchases.changes();
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0], PurchaseChanges::PurchaseDeleted));
    }

    // --- add_items ---------------------------------------------------------

    #[tokio::test]
    async fn add_items_validates_and_calls_repo() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = AddItemsCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            items: vec![input("Apple", 2, 150)],
        };
        svc.add_items(cmd).await.unwrap();
        assert_eq!(svc.purchases.add_items_calls.load(Ordering::SeqCst), 1);
        // add_items builds history inside the repo (sequences assigned there),
        // so the service itself records no changes.
        assert!(svc.purchases.changes().is_empty());
    }

    #[tokio::test]
    async fn add_items_rejects_zero_quantity() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = AddItemsCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            items: vec![input("Apple", 0, 150)],
        };
        let err = svc.add_items(cmd).await.unwrap_err();
        assert!(matches!(err, ApplicationError::Validation(_)));
        assert_eq!(svc.purchases.add_items_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn add_items_rejects_negative_price() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = AddItemsCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            items: vec![input("Apple", 1, -10)],
        };
        let err = svc.add_items(cmd).await.unwrap_err();
        assert!(matches!(err, ApplicationError::Validation(_)));
    }

    // --- update_item -------------------------------------------------------

    #[tokio::test]
    async fn update_item_with_no_change_skips_repo_call() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let item = sample_item(1); // name Milk, price 100, qty 2
        let svc = service(repo, MockItemRepo::new(item.clone()));

        let cmd = UpdateItemCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            sequence: 1,
            name: Some("Milk".to_string()), // same
            unit_price: Some(100),          // same
            quantity: Some(2),              // same
            item_type: Some("grocery".to_string()),
        };
        let result = svc.update_item(cmd).await.unwrap();
        assert_eq!(result.version, 5);
        assert_eq!(svc.purchases.update_item_calls.load(Ordering::SeqCst), 0);
        assert!(svc.purchases.changes().is_empty());
    }

    #[tokio::test]
    async fn update_item_with_change_forwards_item_updated_diff() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let item = sample_item(1); // qty 2
        let svc = service(repo, MockItemRepo::new(item.clone()));

        let cmd = UpdateItemCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            sequence: 1,
            name: None,
            unit_price: None,
            quantity: Some(5),
            item_type: None,
        };
        let result = svc.update_item(cmd).await.unwrap();
        assert_eq!(result.version, 6);
        assert_eq!(svc.purchases.update_item_calls.load(Ordering::SeqCst), 1);
        let changes = svc.purchases.changes();
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            PurchaseChanges::ItemUpdated { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].from.quantity, 2);
                assert_eq!(items[0].to.quantity, 5);
            }
            _ => panic!("expected item_updated"),
        }
    }

    #[tokio::test]
    async fn update_item_rejects_invalid_quantity() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = UpdateItemCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            sequence: 1,
            name: None,
            unit_price: None,
            quantity: Some(0),
            item_type: None,
        };
        let err = svc.update_item(cmd).await.unwrap_err();
        assert!(matches!(err, ApplicationError::Validation(_)));
        assert_eq!(svc.purchases.update_item_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn update_item_rejects_negative_price() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(1)));

        let cmd = UpdateItemCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            sequence: 1,
            name: None,
            unit_price: Some(-50),
            quantity: None,
            item_type: None,
        };
        let err = svc.update_item(cmd).await.unwrap_err();
        assert!(matches!(err, ApplicationError::Validation(_)));
        assert_eq!(svc.purchases.update_item_calls.load(Ordering::SeqCst), 0);
    }

    // --- remove_item -------------------------------------------------------

    #[tokio::test]
    async fn remove_item_records_before_snapshot() {
        let before = sample_purchase(5);
        let repo = MockPurchaseRepo::new(before.clone());
        let svc = service(repo, MockItemRepo::new(sample_item(7)));

        let cmd = RemoveItemCommand {
            id: before.id,
            user_id: before.user_id,
            current_version: 5,
            sequence: 7,
        };
        let result = svc.remove_item(cmd).await.unwrap();
        assert_eq!(result.version, 6);
        assert_eq!(svc.purchases.remove_item_calls.load(Ordering::SeqCst), 1);
        let changes = svc.purchases.changes();
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            PurchaseChanges::ItemRemoved { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].item_sequence, 7);
                assert_eq!(items[0].name, "Milk");
                assert_eq!(items[0].quantity, 2);
            }
            _ => panic!("expected item_removed"),
        }
    }
}
