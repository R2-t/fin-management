use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::LedgerRepository;
use crate::model::ledger::{LedgerEntry, LedgerKind, WalletBalance};
use crate::repository::ledger::NewLedgerEntry;

/// Default number of entries returned by a ledger listing.
pub const DEFAULT_LEDGER_LIMIT: i64 = 50;

/// Maximum number of entries a ledger listing may return.
pub const MAX_LEDGER_LIMIT: i64 = 500;

/// Read and write access to a Wallet's ledger.
///
/// The service owns the rules that must hold no matter which repository
/// implementation is used: the money is signed correctly for the kind of
/// entry, and a listing's bounds are clamped.
pub struct LedgerService<L>
where
    L: LedgerRepository,
{
    ledger: L,
}

impl<L> LedgerService<L>
where
    L: LedgerRepository,
{
    pub fn new(ledger: L) -> Self {
        Self { ledger }
    }

    /// A Wallet's balance: the sum of its entries (ADR-0004).
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %wallet_id))]
    pub async fn balance(
        &self,
        user_id: uuid::Uuid,
        wallet_id: uuid::Uuid,
    ) -> Result<WalletBalance, ApplicationError> {
        self.ledger.balance(user_id, wallet_id).await
    }

    /// A Wallet's entries, newest first.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %wallet_id))]
    pub async fn list(
        &self,
        user_id: uuid::Uuid,
        wallet_id: uuid::Uuid,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<LedgerEntry>, ApplicationError> {
        let limit = limit
            .unwrap_or(DEFAULT_LEDGER_LIMIT)
            .clamp(1, MAX_LEDGER_LIMIT);
        let offset = offset.unwrap_or(0).max(0);
        self.ledger.list(user_id, wallet_id, limit, offset).await
    }

    /// Records an opening balance.
    ///
    /// The amount is entered as a positive magnitude and stored signed, so the
    /// caller cannot accidentally create money by passing a negative number to
    /// a debit-shaped operation.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %wallet_id))]
    pub async fn set_opening_balance(
        &self,
        user_id: uuid::Uuid,
        wallet_id: uuid::Uuid,
        amount_minor: i64,
        currency: String,
        occurred_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<LedgerEntry, ApplicationError> {
        if amount_minor < 0 {
            return Err(ApplicationError::Validation(
                "opening balance cannot be negative".into(),
            ));
        }
        self.ledger
            .append(NewLedgerEntry {
                user_id,
                wallet_id,
                occurred_at,
                amount_minor,
                currency,
                kind: LedgerKind::OpeningBalance,
                purchase_id: None,
                transfer_id: None,
            })
            .await
    }

    /// Records a manual adjustment. A negative amount reduces the balance.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %wallet_id))]
    pub async fn adjust(
        &self,
        user_id: uuid::Uuid,
        wallet_id: uuid::Uuid,
        amount_minor: i64,
        currency: String,
        occurred_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<LedgerEntry, ApplicationError> {
        if amount_minor == 0 {
            return Err(ApplicationError::Validation(
                "an adjustment cannot be zero".into(),
            ));
        }
        self.ledger
            .append(NewLedgerEntry {
                user_id,
                wallet_id,
                occurred_at,
                amount_minor,
                currency,
                kind: LedgerKind::Adjustment,
                purchase_id: None,
                transfer_id: None,
            })
            .await
    }
}
