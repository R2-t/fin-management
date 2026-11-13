use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{LedgerRepository, Result};
use crate::model::ledger::{LedgerEntry, LedgerKind, WalletBalance};
use crate::repository::RepositoryError;

use super::PgPool;

/// A new ledger entry to append.
#[derive(Debug, Clone)]
pub struct NewLedgerEntry {
    pub user_id: Uuid,
    pub wallet_id: Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub amount_minor: i64,
    pub currency: String,
    pub kind: LedgerKind,
    pub purchase_id: Option<Uuid>,
    pub transfer_id: Option<Uuid>,
}

pub struct PgLedgerRepo {
    pool: PgPool,
}

impl PgLedgerRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_entry(row: &tokio_postgres::Row) -> LedgerEntry {
    LedgerEntry {
        id: row.get("id"),
        user_id: row.get("user_id"),
        wallet_id: row.get("wallet_id"),
        occurred_at: row.get("occurred_at"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        kind: LedgerKind::parse(row.get("kind")).unwrap_or(LedgerKind::Adjustment),
        purchase_id: row.get("purchase_id"),
        transfer_id: row.get("transfer_id"),
        superseded_at: row.get("superseded_at"),
        created_at: row.get("created_at"),
    }
}

impl LedgerRepository for PgLedgerRepo {
    /// Appends one entry. Entries are never updated or deleted: a correction
    /// is a new entry that offsets the old one.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "ledger.append"))]
    async fn append(&self, entry: NewLedgerEntry) -> Result<LedgerEntry> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO ledger_entry
                    (user_id, wallet_id, occurred_at, amount_minor, currency, kind, purchase_id, transfer_id)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                   RETURNING *"#,
                &[
                    &entry.user_id,
                    &entry.wallet_id,
                    &entry.occurred_at,
                    &entry.amount_minor,
                    &entry.currency,
                    &entry.kind.as_str(),
                    &entry.purchase_id,
                    &entry.transfer_id,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_entry(&row))
    }

    /// A Wallet's balance: the sum of its entries (ADR-0004).
    ///
    /// The currency comes from the wallet, not from the entries, so the caller
    /// always sees the Wallet's own currency even when it has no entries yet.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "ledger.balance"))]
    async fn balance(&self, user_id: Uuid, wallet_id: Uuid) -> Result<WalletBalance> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                r#"SELECT w.currency AS currency,
                          COALESCE(SUM(l.amount_minor), 0)::bigint AS balance_minor,
                          COUNT(l.id)::bigint AS entry_count
                     FROM wallet w
                     LEFT JOIN ledger_entry l
                       ON l.wallet_id = w.id AND l.user_id = $2 AND l.superseded_at IS NULL
                    WHERE w.id = $1 AND w.user_id = $2 AND w.deleted_at IS NULL
                    GROUP BY w.currency"#,
                &[&wallet_id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(WalletBalance {
            wallet_id,
            currency: row.get("currency"),
            balance_minor: row.get("balance_minor"),
            entry_count: row.get("entry_count"),
        })
    }

    /// A Wallet's entries, newest first.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "ledger.list"))]
    async fn list(
        &self,
        user_id: Uuid,
        wallet_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LedgerEntry>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        // The wallet must belong to the caller. Without this check a request
        // for someone else's wallet returns an empty list, which reads as
        // "this wallet has no entries" rather than "not yours".
        let owns_wallet = conn
            .query_opt(
                "SELECT 1 FROM wallet WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&wallet_id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        if owns_wallet.is_none() {
            return Err(RepositoryError::NotFound.into());
        }

        let rows = conn
            .query(
                "SELECT * FROM ledger_entry
                  WHERE wallet_id = $1 AND user_id = $2 AND superseded_at IS NULL
                  ORDER BY occurred_at DESC, created_at DESC
                  LIMIT $3 OFFSET $4",
                &[&wallet_id, &user_id, &limit, &offset],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_entry).collect())
    }
}
