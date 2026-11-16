use tracing::instrument;
use uuid::Uuid;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{Result, WalletRepository};
use crate::model::wallet::Wallet;
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgWalletRepo {
    pool: PgPool,
}

impl PgWalletRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_wallet(row: &tokio_postgres::Row) -> Wallet {
    Wallet {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        currency: row.get("currency"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}

impl WalletRepository for PgWalletRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "wallet.create"))]
    async fn create(&self, user_id: Uuid, name: String, currency: String) -> Result<Wallet> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                "INSERT INTO wallet (id, user_id, name, currency) VALUES ($1, $2, $3, $4) RETURNING *",
                &[&Uuid::new_v4(), &user_id, &name, &currency],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_wallet(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "wallet.list"))]
    async fn list(&self, user_id: Uuid) -> Result<Vec<Wallet>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                "SELECT * FROM wallet WHERE user_id = $1 AND deleted_at IS NULL ORDER BY name",
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_wallet).collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "wallet.get"))]
    async fn get(&self, user_id: Uuid, id: Uuid) -> Result<Wallet> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT * FROM wallet WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_wallet(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "wallet.update"))]
    async fn update(
        &self,
        id: Uuid,
        user_id: Uuid,
        name: String,
        currency: Option<String>,
    ) -> Result<Wallet> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        // Lock the row so two concurrent currency changes cannot interleave
        // their cascades.
        let current = tran
            .query_opt(
                "SELECT currency FROM wallet
                  WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
                  FOR UPDATE",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;
        let old_currency: String = current.get("currency");
        let changing = currency.as_deref().is_some_and(|c| c != old_currency);

        if changing {
            // D50. A wallet that holds entries keeps its currency. Converting
            // them would need a rate for a date in the past that was never
            // recorded, and inventing one is what this app refuses to do.
            // The one-currency-per-wallet invariant is what keeps the balance,
            // the runway and every budget reading a single number.
            let has_entries: bool = tran
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM ledger_entry WHERE wallet_id = $1)",
                    &[&id],
                )
                .await
                .map_err(RepositoryError::DbError)?
                .get("exists");

            if has_entries {
                return Err(ApplicationError::Validation(format!(
                    "wallet holds a balance in {old_currency}; create a new wallet instead of changing its currency"
                )));
            }

            // D32 and D51: what the wallet denominated in the old currency
            // stops applying. Budget limits, income amounts and recurring
            // charges are all stated in that currency.
            //
            // `recurring_rule` carries `status` rather than `active`, so it
            // is cancelled rather than deactivated.
            tran.execute(
                "UPDATE budget SET active = FALSE, updated_at = NOW()
                  WHERE wallet_id = $1 AND deleted_at IS NULL",
                &[&id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
            tran.execute(
                // `income_schedule` has no `wallet_id` of its own; the wallet
                // it pays into is on its allocations.
                "UPDATE income_schedule SET active = FALSE, updated_at = NOW()
                  WHERE deleted_at IS NULL
                    AND id IN (SELECT income_schedule_id
                                 FROM income_allocation WHERE wallet_id = $1)",
                &[&id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
            tran.execute(
                "UPDATE recurring_rule SET status = 'cancelled', updated_at = NOW()
                  WHERE wallet_id = $1 AND deleted_at IS NULL AND status = 'active'",
                &[&id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        }

        let row = tran
            .query_opt(
                "UPDATE wallet SET name = $1, currency = COALESCE($4, currency), updated_at = NOW() \
                 WHERE id = $2 AND user_id = $3 AND deleted_at IS NULL RETURNING *",
                &[&name, &id, &user_id, &currency],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok(row_to_wallet(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "wallet.delete"))]
    async fn delete(&self, id: Uuid, user_id: Uuid) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let n = conn
            .execute(
                "UPDATE wallet SET deleted_at = NOW() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        if n == 0 {
            return Err(RepositoryError::NotFound.into());
        }
        Ok(())
    }
}
