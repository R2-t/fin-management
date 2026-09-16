use uuid::Uuid;

use crate::application::inbound::repositories::{ItemInput, ItemRepository, ItemUpdate, Result};
use crate::model::item::Item;
use crate::repository::RepositoryError;

use super::PgPool;

#[derive(Clone)]
pub struct PgItemRepo {
    pool: PgPool,
}

impl PgItemRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// SELECT with an explicit cast of NUMERIC → BIGINT so we can map into i64 minor units.
const SELECT_ITEM_COLS: &str = r#"
    item_sequence,
    purchase_id,
    name,
    unit_price::bigint AS unit_price,
    quantity,
    item_type
"#;

fn row_to_item(row: &tokio_postgres::Row) -> Item {
    Item {
        item_sequence: row.get("item_sequence"),
        purchase_id: row.get("purchase_id"),
        name: row.get("name"),
        unit_price: row.get("unit_price"),
        quantity: row.get("quantity"),
        item_type: row.get("item_type"),
    }
}

/// Returns the next item_sequence for the given purchase, starting at 1.
async fn next_sequence(
    conn: &tokio_postgres::Client,
    purchase_id: Uuid,
) -> std::result::Result<i32, RepositoryError> {
    let row = conn
        .query_one(
            "SELECT COALESCE(MAX(item_sequence), 0) + 1 AS next_seq FROM item WHERE purchase_id = $1",
            &[&purchase_id],
        )
        .await
        .map_err(RepositoryError::DbError)?;
    Ok(row.get("next_seq"))
}

impl ItemRepository for PgItemRepo {
    async fn create(&self, purchase_id: Uuid, item: ItemInput) -> Result<Item> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let seq = next_sequence(&conn, purchase_id).await?;

        let row = conn
            .query_one(
                &format!(
                    r#"INSERT INTO item (item_sequence, purchase_id, name, unit_price, quantity, item_type)
                       VALUES ($1, $2, $3, $4, $5, $6)
                       RETURNING {SELECT_ITEM_COLS}"#
                ),
                &[
                    &seq,
                    &purchase_id,
                    &item.name,
                    &item.unit_price,
                    &item.quantity,
                    &item.item_type,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_item(&row))
    }

    async fn bulk_create(&self, purchase_id: Uuid, items: Vec<ItemInput>) -> Result<Vec<Item>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let base_seq = next_sequence(&conn, purchase_id).await?;

        let mut created = Vec::with_capacity(items.len());

        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        for (i, item) in items.into_iter().enumerate() {
            let seq = base_seq + i as i32;
            let row = tran
                .query_one(
                    &format!(
                        r#"INSERT INTO item (item_sequence, purchase_id, name, unit_price, quantity, item_type)
                           VALUES ($1, $2, $3, $4, $5, $6)
                           RETURNING {SELECT_ITEM_COLS}"#
                    ),
                    &[
                        &seq,
                        &purchase_id,
                        &item.name,
                        &item.unit_price,
                        &item.quantity,
                        &item.item_type,
                    ],
                )
                .await
                .map_err(RepositoryError::DbError)?;
            created.push(row_to_item(&row));
        }

        tran.commit().await.map_err(RepositoryError::DbError)?;
        Ok(created)
    }

    async fn get(&self, purchase_id: Uuid, sequence: i32) -> Result<Item> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                &format!(
                    "SELECT {SELECT_ITEM_COLS} FROM item WHERE purchase_id = $1 AND item_sequence = $2"
                ),
                &[&purchase_id, &sequence],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_item(&row))
    }

    async fn update(&self, purchase_id: Uuid, sequence: i32, item: ItemUpdate) -> Result<Item> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                &format!(
                    r#"UPDATE item SET
                        name       = COALESCE($1, name),
                        unit_price = COALESCE($2, unit_price),
                        quantity   = COALESCE($3, quantity),
                        item_type  = COALESCE($4, item_type)
                       WHERE purchase_id = $5 AND item_sequence = $6
                       RETURNING {SELECT_ITEM_COLS}"#
                ),
                &[
                    &item.name,
                    &item.unit_price,
                    &item.quantity,
                    &item.item_type,
                    &purchase_id,
                    &sequence,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_item(&row))
    }

    async fn delete(&self, purchase_id: Uuid, sequence: i32) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let n = conn
            .execute(
                "DELETE FROM item WHERE purchase_id = $1 AND item_sequence = $2",
                &[&purchase_id, &sequence],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        if n == 0 {
            return Err(RepositoryError::NotFound.into());
        }
        Ok(())
    }

    async fn list_by_purchase(&self, purchase_id: Uuid) -> Result<Vec<Item>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                &format!(
                    "SELECT {SELECT_ITEM_COLS} FROM item WHERE purchase_id = $1 ORDER BY item_sequence"
                ),
                &[&purchase_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_item).collect())
    }
}
