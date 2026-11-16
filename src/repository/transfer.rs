use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{Result, TransferRepository};
use crate::model::transfer::Transfer;
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgTransferRepo {
    pool: PgPool,
}

impl PgTransferRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_transfer(row: &tokio_postgres::Row) -> Transfer {
    Transfer {
        id: row.get("id"),
        user_id: row.get("user_id"),
        from_wallet_id: row.get("from_wallet_id"),
        to_wallet_id: row.get("to_wallet_id"),
        occurred_at: row.get("occurred_at"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        description: row.get("description"),
        created_at: row.get("created_at"),
        deleted_at: row.get("deleted_at"),
    }
}

impl TransferRepository for PgTransferRepo {
    /// Records a transfer and writes both ledger legs in the same transaction.
    ///
    /// The legs are written here rather than by the caller, because they must
    /// net to zero: a partial write would silently create or destroy money in
    /// aggregate figures (ADR-0004).
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "transfer.create"))]
    async fn create(&self, transfer: Transfer) -> Result<Transfer> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let row = tran
            .query_one(
                r#"INSERT INTO transfer
                    (id, user_id, from_wallet_id, to_wallet_id, occurred_at, amount_minor, currency, description)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                   RETURNING *"#,
                &[
                    &transfer.id,
                    &transfer.user_id,
                    &transfer.from_wallet_id,
                    &transfer.to_wallet_id,
                    &transfer.occurred_at,
                    &transfer.amount_minor,
                    &transfer.currency,
                    &transfer.description,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        // The two legs. They net to zero by construction: one is the negation
        // of the other.
        for (wallet_id, amount) in [
            (transfer.from_wallet_id, -transfer.amount_minor),
            (transfer.to_wallet_id, transfer.amount_minor),
        ] {
            tran.execute(
                r#"INSERT INTO ledger_entry
                       (user_id, wallet_id, occurred_at, amount_minor, currency, kind, transfer_id)
                   VALUES ($1, $2, $3, $4, $5, 'transfer', $6)"#,
                &[
                    &transfer.user_id,
                    &wallet_id,
                    &transfer.occurred_at,
                    &amount,
                    &transfer.currency,
                    &transfer.id,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        }

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok(row_to_transfer(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "transfer.list"))]
    async fn list(&self, user_id: Uuid) -> Result<Vec<Transfer>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                "SELECT * FROM transfer WHERE user_id = $1 AND deleted_at IS NULL ORDER BY occurred_at DESC, created_at DESC",
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_transfer).collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "transfer.get"))]
    async fn get(&self, user_id: Uuid, id: Uuid) -> Result<Transfer> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT * FROM transfer WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_transfer(&row))
    }

    /// Deletes a transfer and reverses its legs.
    ///
    /// The legs are superseded rather than deleted, so the money's history is
    /// preserved: the balance sums active entries only.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "transfer.delete"))]
    async fn delete(&self, user_id: Uuid, id: Uuid) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let n = tran
            .execute(
                "UPDATE transfer SET deleted_at = NOW() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        if n == 0 {
            return Err(RepositoryError::NotFound.into());
        }

        tran.execute(
            "UPDATE ledger_entry SET superseded_at = NOW() WHERE transfer_id = $1 AND user_id = $2 AND superseded_at IS NULL",
            &[&id, &user_id],
        )
        .await
        .map_err(RepositoryError::DbError)?;

        tran.commit().await.map_err(RepositoryError::DbError)?;
        Ok(())
    }
}
