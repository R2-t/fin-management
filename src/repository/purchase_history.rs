use uuid::Uuid;

use crate::application::inbound::repositories::{PurchaseHistoryRepository, Result};
use crate::model::purchase_history::PurchaseHistory;
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgPurchaseHistoryRepo {
    pool: PgPool,
}

impl PgPurchaseHistoryRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_history(row: &tokio_postgres::Row) -> PurchaseHistory {
    PurchaseHistory {
        id: row.get("id"),
        purchase_id: row.get("purchase_id"),
        version: row.get("version"),
        changes: row.get("changes"),
        changed_at: row.get("changed_at"),
    }
}

impl PurchaseHistoryRepository for PgPurchaseHistoryRepo {
    async fn insert(
        &self,
        purchase_id: Uuid,
        version: i32,
        changes: serde_json::Value,
    ) -> Result<PurchaseHistory> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO purchase_history (purchase_id, version, changes)
                   VALUES ($1, $2, $3)
                   RETURNING *"#,
                &[&purchase_id, &version, &changes],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_history(&row))
    }

    async fn get_for_purchase(&self, purchase_id: Uuid) -> Result<Vec<PurchaseHistory>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                "SELECT * FROM purchase_history WHERE purchase_id = $1 ORDER BY version DESC",
                &[&purchase_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_history).collect())
    }
}
