use tracing::instrument;

use crate::application::{
    currency::{normalize_currency, DEFAULT_CURRENCY},
    error::ApplicationError,
    inbound::repositories::WalletRepository,
};
use crate::model::wallet::Wallet;

pub struct WalletService<W>
where
    W: WalletRepository,
{
    wallets: W,
}

impl<W> WalletService<W>
where
    W: WalletRepository,
{
    pub fn new(wallets: W) -> Self {
        Self { wallets }
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn create(
        &self,
        user_id: uuid::Uuid,
        name: String,
        currency: Option<String>,
    ) -> Result<Wallet, ApplicationError> {
        if name.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "wallet name cannot be empty".into(),
            ));
        }
        let currency = match currency {
            Some(c) => normalize_currency(&c)?,
            None => DEFAULT_CURRENCY.to_string(),
        };
        self.wallets.create(user_id, name, currency).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Wallet>, ApplicationError> {
        self.wallets.list(user_id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %id))]
    pub async fn get(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<Wallet, ApplicationError> {
        self.wallets.get(user_id, id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %id))]
    pub async fn update(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
        name: String,
        currency: Option<String>,
    ) -> Result<Wallet, ApplicationError> {
        if name.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "wallet name cannot be empty".into(),
            ));
        }
        let currency = currency.map(|c| normalize_currency(&c)).transpose()?;
        self.wallets.update(id, user_id, name, currency).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %id))]
    pub async fn delete(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.wallets.delete(id, user_id).await
    }
}
