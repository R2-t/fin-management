use tracing::instrument;
use uuid::Uuid;

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
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "UPDATE wallet SET name = $1, currency = COALESCE($4, currency), updated_at = NOW() \
                 WHERE id = $2 AND user_id = $3 AND deleted_at IS NULL RETURNING *",
                &[&name, &id, &user_id, &currency],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

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
