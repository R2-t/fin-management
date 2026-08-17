use uuid::Uuid;

use crate::application::inbound::repositories::{
    PurchaseFilters, PurchaseRepository, PurchaseUpdates, Result,
};
use crate::model::purchase::Purchase;
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgPurchaseRepo {
    pool: PgPool,
}

impl PgPurchaseRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_purchase(row: &tokio_postgres::Row) -> Purchase {
    Purchase {
        id: row.get("id"),
        user_id: row.get("user_id"),
        wallet_id: row.get("wallet_id"),
        category_id: row.get("category_id"),
        store: row.get("store"),
        description: row.get("description"),
        notes: row.get("notes"),
        version: row.get("version"),
        purchase_date: row.get("purchase_date"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}

impl PurchaseRepository for PgPurchaseRepo {
    async fn create(
        &self,
        wallet_id: Uuid,
        store: Option<String>,
        notes: Option<String>,
        user_id: Uuid,
        category_id: i32,
        purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Purchase> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let purchase_date = purchase_date.unwrap_or_else(chrono::Utc::now);

        let row = conn
            .query_one(
                r#"INSERT INTO purchase
                    (id, user_id, wallet_id, category_id, store, notes, purchase_date)
                   VALUES ($1, $2, $3, $4, $5, $6, $7)
                   RETURNING *"#,
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &wallet_id,
                    &category_id,
                    &store,
                    &notes,
                    &purchase_date,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_purchase(&row))
    }

    async fn get(&self, id: Uuid, user_id: Uuid) -> Result<Purchase> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT * FROM purchase WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_purchase(&row))
    }

    async fn list(&self, user_id: Uuid, filters: PurchaseFilters) -> Result<Vec<Purchase>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let limit = filters.limit.unwrap_or(50);
        let offset = filters.offset.unwrap_or(0);
        let start_date = filters
            .start_date
            .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(30));

        let rows = conn
            .query(
                r#"SELECT * FROM purchase
                   WHERE user_id = $1
                     AND deleted_at IS NULL
                     AND purchase_date >= $2
                     AND ($3::timestamptz IS NULL OR purchase_date <= $3)
                     AND ($4::uuid IS NULL OR wallet_id = $4)
                     AND ($5::integer IS NULL OR category_id = $5)
                   ORDER BY purchase_date DESC
                   LIMIT $6 OFFSET $7"#,
                &[
                    &user_id,
                    &start_date,
                    &filters.end_date,
                    &filters.wallet_id,
                    &filters.category_id,
                    &limit,
                    &offset,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_purchase).collect())
    }

    async fn update(
        &self,
        purchase_id: Uuid,
        user_id: Uuid,
        current_version: i32,
        purchase_update: PurchaseUpdates,
    ) -> Result<Purchase> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                r#"UPDATE purchase SET
                    store       = COALESCE($1, store),
                    description = COALESCE($2, description),
                    notes       = COALESCE($3, notes),
                    category_id = COALESCE($4, category_id),
                    version     = version + 1,
                    updated_at  = NOW()
                   WHERE id = $5 AND user_id = $6 AND version = $7 AND deleted_at IS NULL
                   RETURNING *"#,
                &[
                    &purchase_update.store,
                    &purchase_update.description,
                    &purchase_update.notes,
                    &purchase_update.category_id,
                    &purchase_id,
                    &user_id,
                    &current_version,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_purchase(&row))
    }

    async fn delete(&self, id: Uuid, user_id: Uuid) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let n = conn
            .execute(
                "UPDATE purchase SET deleted_at = NOW() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
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
