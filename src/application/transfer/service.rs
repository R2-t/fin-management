use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{TransferRepository, WalletRepository};
use crate::model::transfer::Transfer;

/// The caller's intent for a new transfer.
///
/// Bundled as one named type rather than seven positional arguments: this
/// codebase has already had one real argument-order defect from same-typed
/// positionals, and `from_wallet_id`/`to_wallet_id` are the same type.
#[derive(Debug, Clone)]
pub struct CreateTransfer {
    pub from_wallet_id: uuid::Uuid,
    pub to_wallet_id: uuid::Uuid,
    pub amount_minor: i64,
    pub currency: Option<String>,
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    pub description: Option<String>,
}

/// Records transfers between the user's own Wallets.
///
/// A transfer is a first-class object that generates its own ledger legs
/// (ADR-0003). The service owns the rules that make a transfer coherent: the
/// two wallets must be the caller's, must differ, and must hold the same
/// currency -- a transfer is a movement of money between pots of the same
/// money, not a currency exchange.
pub struct TransferService<T, W>
where
    T: TransferRepository,
    W: WalletRepository,
{
    transfers: T,
    wallets: W,
}

impl<T, W> TransferService<T, W>
where
    T: TransferRepository,
    W: WalletRepository,
{
    pub fn new(transfers: T, wallets: W) -> Self {
        Self { transfers, wallets }
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Transfer>, ApplicationError> {
        self.transfers.list(user_id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, transfer_id = %id))]
    pub async fn get(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<Transfer, ApplicationError> {
        self.transfers.get(user_id, id).await
    }

    /// Records a transfer between two of the user's Wallets.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn create(
        &self,
        user_id: uuid::Uuid,
        input: CreateTransfer,
    ) -> Result<Transfer, ApplicationError> {
        let CreateTransfer {
            from_wallet_id,
            to_wallet_id,
            amount_minor,
            currency,
            occurred_at,
            description,
        } = input;

        if from_wallet_id == to_wallet_id {
            return Err(ApplicationError::Validation(
                "a transfer must move money between two different wallets".into(),
            ));
        }
        if amount_minor <= 0 {
            return Err(ApplicationError::Validation(
                "a transfer amount must be greater than zero".into(),
            ));
        }

        // Both wallets must belong to the caller. Loading them also gives the
        // currency, which must be the same on both sides.
        let from = self.wallets.get(user_id, from_wallet_id).await?;
        let to = self.wallets.get(user_id, to_wallet_id).await?;

        if from.currency != to.currency {
            return Err(ApplicationError::Validation(format!(
                "cannot transfer between {} and {}: a transfer moves money within one currency, not between currencies",
                from.currency, to.currency
            )));
        }

        let currency = match currency {
            Some(c) => crate::application::currency::normalize_currency(&c)?,
            None => from.currency.clone(),
        };
        if currency != from.currency {
            return Err(ApplicationError::Validation(format!(
                "the wallets hold {}, so the transfer cannot be in {currency}",
                from.currency
            )));
        }

        self.transfers
            .create(Transfer {
                id: uuid::Uuid::new_v4(),
                user_id,
                from_wallet_id,
                to_wallet_id,
                occurred_at: occurred_at.unwrap_or_else(chrono::Utc::now),
                amount_minor,
                currency,
                description,
                created_at: chrono::Utc::now(),
                deleted_at: None,
            })
            .await
    }

    /// Deletes a transfer and reverses its ledger legs.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, transfer_id = %id))]
    pub async fn delete(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.transfers.delete(user_id, id).await
    }
}
