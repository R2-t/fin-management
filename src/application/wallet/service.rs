use tracing::instrument;

use crate::application::{error::ApplicationError, inbound::repositories::WalletRepository};
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

    #[instrument(skip(self), fields(user_id = %user_id))]
    pub async fn create(
        &self,
        user_id: uuid::Uuid,
        name: String,
    ) -> Result<Wallet, ApplicationError> {
        if name.trim().is_empty() {
            return Err(ApplicationError::Validation("wallet name cannot be empty".into()));
        }
        self.wallets.create(user_id, name).await
    }

    #[instrument(skip(self), fields(user_id = %user_id))]
    pub async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Wallet>, ApplicationError> {
        self.wallets.list(user_id).await
    }

    #[instrument(skip(self), fields(user_id = %user_id, wallet_id = %id))]
    pub async fn get(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<Wallet, ApplicationError> {
        self.wallets.get(user_id, id).await
    }

    #[instrument(skip(self), fields(user_id = %user_id, wallet_id = %id))]
    pub async fn update(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
        name: String,
    ) -> Result<Wallet, ApplicationError> {
        if name.trim().is_empty() {
            return Err(ApplicationError::Validation("wallet name cannot be empty".into()));
        }
        self.wallets.update(id, user_id, name).await
    }

    #[instrument(skip(self), fields(user_id = %user_id, wallet_id = %id))]
    pub async fn delete(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.wallets.delete(id, user_id).await
    }
}
