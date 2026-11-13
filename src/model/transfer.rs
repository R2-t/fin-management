use serde::{Deserialize, Serialize};

/// A transfer of money between two of the user's own Wallets.
///
/// A first-class record (ADR-0003): it generates its own ledger legs, so
/// correcting a mistaken transfer is a coherent edit of one object rather than
/// a hunt for two rows that must agree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub from_wallet_id: uuid::Uuid,
    pub to_wallet_id: uuid::Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    /// Positive: the magnitude moved. The legs carry the signs.
    pub amount_minor: i64,
    pub currency: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}
