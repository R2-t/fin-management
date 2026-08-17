use uuid::Uuid;

use crate::application::inbound::repositories::{
    CreatePurchaseParams, ItemInput, PurchaseFilters, PurchaseRepository, PurchaseUpdates, Result,
};
use crate::model::item::Item;
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
        currency: row.get("currency"),
        exchange_rate: row.get("exchange_rate"),
        rate_source: row.get("rate_source"),
        rate_at: row.get("rate_at"),
        purchase_date: row.get("purchase_date"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}

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

async fn next_item_sequence(
    tran: &tokio_postgres::Transaction<'_>,
    purchase_id: Uuid,
) -> std::result::Result<i32, RepositoryError> {
    let row = tran
        .query_one(
            "SELECT COALESCE(MAX(item_sequence), 0) + 1 AS next_seq FROM item WHERE purchase_id = $1",
            &[&purchase_id],
        )
        .await
        .map_err(RepositoryError::DbError)?;
    Ok(row.get("next_seq"))
}

impl PurchaseRepository for PgPurchaseRepo {
    async fn create(&self, params: CreatePurchaseParams) -> Result<Purchase> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let purchase_date = params.purchase_date.unwrap_or_else(chrono::Utc::now);

        let row = conn
            .query_one(
                r#"INSERT INTO purchase
                    (id, user_id, wallet_id, category_id, store, notes, purchase_date,
                     currency, exchange_rate, rate_source, rate_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                   RETURNING *"#,
                &[
                    &Uuid::new_v4(),
                    &params.user_id,
                    &params.wallet_id,
                    &params.category_id,
                    &params.store,
                    &params.notes,
                    &purchase_date,
                    &params.currency,
                    &params.exchange_rate,
                    &params.rate_source,
                    &params.rate_at,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_purchase(&row))
    }

    async fn create_with_items(
        &self,
        params: CreatePurchaseParams,
        items: Vec<ItemInput>,
    ) -> Result<(Purchase, Vec<Item>)> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let purchase_date = params.purchase_date.unwrap_or_else(chrono::Utc::now);

        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let purchase_id = Uuid::new_v4();

        let row = tran
            .query_one(
                r#"INSERT INTO purchase
                    (id, user_id, wallet_id, category_id, store, notes, purchase_date,
                     currency, exchange_rate, rate_source, rate_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                   RETURNING *"#,
                &[
                    &purchase_id,
                    &params.user_id,
                    &params.wallet_id,
                    &params.category_id,
                    &params.store,
                    &params.notes,
                    &purchase_date,
                    &params.currency,
                    &params.exchange_rate,
                    &params.rate_source,
                    &params.rate_at,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        let purchase = row_to_purchase(&row);

        let created_items = if items.is_empty() {
            Vec::new()
        } else {
            let base_seq = next_item_sequence(&tran, purchase_id).await?;
            let mut created = Vec::with_capacity(items.len());

            for (i, item) in items.into_iter().enumerate() {
                let seq = base_seq + i as i32;
                let item_row = tran
                    .query_one(
                        r#"INSERT INTO item
                            (item_sequence, purchase_id, name, unit_price, quantity, item_type)
                           VALUES ($1, $2, $3, $4, $5, $6)
                           RETURNING item_sequence, purchase_id, name, unit_price::bigint AS unit_price, quantity, item_type"#,
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
                created.push(row_to_item(&item_row));
            }

            created
        };

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok((purchase, created_items))
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
