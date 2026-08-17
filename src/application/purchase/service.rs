use chrono::Timelike;
use chrono::{DateTime, Utc};
use tracing::{instrument, warn};

use crate::application::{
    error::ApplicationError,
    inbound::repositories::{
        PurchaseFilters, PurchaseRepository, PurchaseUpdates, RateCacheRepository, WalletRepository,
    },
    outbound::exchange_rate::{ExchangeRateProvider, RateRequest},
};
use crate::model::purchase::Purchase;

use super::commands::{CreatePurchaseCommand, UpdatePurchaseCommand};

const CURRENCY_CODE_LEN: usize = 3;

pub struct PurchaseService<P, R, W, F>
where
    P: PurchaseRepository,
    R: RateCacheRepository,
    W: WalletRepository,
    F: ExchangeRateProvider,
{
    purchases: P,
    rate_cache: R,
    wallets: W,
    fx: F,
}

impl<P, R, W, F> PurchaseService<P, R, W, F>
where
    P: PurchaseRepository,
    R: RateCacheRepository,
    W: WalletRepository,
    F: ExchangeRateProvider,
{
    pub fn new(purchases: P, rate_cache: R, wallets: W, fx: F) -> Self {
        Self {
            purchases,
            rate_cache,
            wallets,
            fx,
        }
    }

    #[instrument(skip(self), fields(user_id = %cmd.user_id, wallet_id = %cmd.wallet_id))]
    pub async fn create(&self, cmd: CreatePurchaseCommand) -> Result<Purchase, ApplicationError> {
        // 1. Input validation
        let currency = cmd.currency.to_uppercase();
        validate_currency(&currency)?;
        validate_items(&cmd.items)?;

        // 2. Load wallet to determine its holding currency
        let wallet = self.wallets.get(cmd.wallet_id, cmd.user_id).await?;

        // 3. Rate resolution
        let (exchange_rate, rate_source, rate_at) = self
            .resolve_rate(&cmd, &currency, &wallet.currency, cmd.purchase_date)
            .await?;

        // 4. Atomic persist (purchase + items in one transaction)
        let (purchase, _items) = self
            .purchases
            .create_with_items(
                cmd.wallet_id,
                cmd.store,
                cmd.notes,
                cmd.user_id,
                cmd.category_id,
                cmd.purchase_date,
                currency,
                exchange_rate,
                rate_source,
                rate_at,
                cmd.items,
            )
            .await?;

        Ok(purchase)
    }

    async fn resolve_rate(
        &self,
        cmd: &CreatePurchaseCommand,
        source_currency: &str,
        wallet_currency: &str,
        purchase_date: Option<DateTime<Utc>>,
    ) -> Result<(Option<f64>, Option<String>, Option<DateTime<Utc>>), ApplicationError> {
        // Manual override wins: warn + flag, no cache/API.
        if let Some(override_rate) = cmd.exchange_rate_override {
            if override_rate <= 0.0 {
                return Err(ApplicationError::Validation(
                    "exchange_rate_override must be greater than zero".into(),
                ));
            }
            warn!(
                source_currency = %source_currency,
                wallet_currency = %wallet_currency,
                rate = %override_rate,
                "manual exchange rate override applied"
            );
            return Ok((
                Some(override_rate),
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
            return Ok((
                Some(cached.rate),
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

        match self.fx.fetch_rate(req).await {
            Ok(result) => {
                self.rate_cache
                    .put(
                        source_currency,
                        wallet_currency,
                        rate_hour,
                        result.rate,
                        &result.source,
                    )
                    .await?;
                Ok((Some(result.rate), Some(result.source), Some(rate_hour)))
            }
            Err(_) => {
                // Provider failed: fall back to most recent cached rate of any age.
                if let Some(prev) = self
                    .rate_cache
                    .get_most_recent(source_currency, wallet_currency)
                    .await?
                {
                    warn!(
                        source_currency = %source_currency,
                        wallet_currency = %wallet_currency,
                        rate = %prev.rate,
                        rate_hour = %prev.rate_hour,
                        "exchange rate provider unavailable; using stale cached rate"
                    );
                    Ok((Some(prev.rate), Some(prev.source), Some(prev.rate_hour)))
                } else {
                    Err(ApplicationError::Internal(format!(
                        "exchange rate unavailable for {source_currency}/{wallet_currency}"
                    )))
                }
            }
        }
    }

    #[instrument(skip(self), fields(purchase_id = %id, user_id = %user_id))]
    pub async fn get(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<Purchase, ApplicationError> {
        let purchase = self.purchases.get(id, user_id).await?;
        Ok(purchase)
    }

    #[instrument(skip(self), fields(user_id = %user_id))]
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

    #[instrument(skip(self), fields(purchase_id = %cmd.id, user_id = %cmd.user_id))]
    pub async fn update(&self, cmd: UpdatePurchaseCommand) -> Result<Purchase, ApplicationError> {
        let updates = PurchaseUpdates {
            id: cmd.id,
            store: cmd.store,
            description: cmd.description,
            notes: cmd.notes,
            category_id: cmd.category_id,
        };

        self.purchases
            .update(cmd.id, cmd.user_id, cmd.current_version, updates)
            .await
    }

    #[instrument(skip(self), fields(purchase_id = %id, user_id = %user_id))]
    pub async fn delete(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.purchases.delete(id, user_id).await
    }
}

fn validate_currency(currency: &str) -> Result<(), ApplicationError> {
    if currency.len() != CURRENCY_CODE_LEN || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApplicationError::Validation(
            "currency must be a 3-letter ISO 4217 code".into(),
        ));
    }
    Ok(())
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
