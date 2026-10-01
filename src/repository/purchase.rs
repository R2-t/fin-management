use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{
    CreatePurchaseParams, ItemInput, ItemUpdate, PurchaseFilters, PurchaseRepository,
    PurchaseUpdates, Result,
};
use crate::application::purchase::diff::{ItemSnapshot, PurchaseChanges};
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
        classification_status: row.get("classification_status"),
        merchant_id: row.get("merchant_id"),
        store: row.get("store"),
        description: row.get("description"),
        notes: row.get("notes"),
        version: row.get("version"),
        currency: row.get("currency"),
        exchange_rate_scaled: row.get("exchange_rate_scaled"),
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

/// Explains why a version-guarded UPDATE matched no row: the purchase is gone
/// (or not owned by the user) => `NotFound`, otherwise the caller's version is
/// stale => `VersionConflict`.
async fn miss_reason(
    tran: &tokio_postgres::Transaction<'_>,
    purchase_id: Uuid,
    user_id: Uuid,
) -> RepositoryError {
    match tran
        .query_opt(
            "SELECT 1 FROM purchase WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
            &[&purchase_id, &user_id],
        )
        .await
    {
        Ok(Some(_)) => RepositoryError::VersionConflict,
        Ok(None) => RepositoryError::NotFound,
        Err(e) => RepositoryError::DbError(e),
    }
}

/// Bumps the purchase version under the optimistic-lock guard and returns the
/// updated purchase. Used by item mutations to record history against the new
/// version in the same transaction.
async fn bump_purchase_version(
    tran: &tokio_postgres::Transaction<'_>,
    purchase_id: Uuid,
    user_id: Uuid,
    current_version: i32,
) -> std::result::Result<Purchase, RepositoryError> {
    let row = tran
        .query_opt(
            r#"UPDATE purchase SET version = version + 1, updated_at = NOW()
               WHERE id = $1 AND user_id = $2 AND version = $3 AND deleted_at IS NULL
               RETURNING *"#,
            &[&purchase_id, &user_id, &current_version],
        )
        .await
        .map_err(RepositoryError::DbError)?;

    match row {
        Some(row) => Ok(row_to_purchase(&row)),
        None => Err(miss_reason(tran, purchase_id, user_id).await),
    }
}

/// Inserts a history row inside an existing transaction.
async fn insert_history(
    tran: &tokio_postgres::Transaction<'_>,
    purchase_id: Uuid,
    version: i32,
    changes: &PurchaseChanges,
) -> std::result::Result<(), RepositoryError> {
    tran.execute(
        r#"INSERT INTO purchase_history (purchase_id, version, changes) VALUES ($1, $2, $3)"#,
        &[&purchase_id, &version, &serde_json::to_value(changes)?],
    )
    .await
    .map_err(RepositoryError::DbError)?;
    Ok(())
}

impl PurchaseRepository for PgPurchaseRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.create"))]
    async fn create(&self, params: CreatePurchaseParams) -> Result<Purchase> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let purchase_date = params.purchase_date.unwrap_or_else(chrono::Utc::now);

        let row = conn
            .query_one(
                r#"INSERT INTO purchase
                    (id, user_id, wallet_id, category_id, merchant_id, store, description, notes,
                     purchase_date, currency, exchange_rate_scaled, rate_source, rate_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                   RETURNING *"#,
                &[
                    &Uuid::new_v4(),
                    &params.user_id,
                    &params.wallet_id,
                    &params.category_id,
                    &params.merchant_id,
                    &params.store,
                    &params.description,
                    &params.notes,
                    &purchase_date,
                    &params.currency,
                    &params.exchange_rate_scaled,
                    &params.rate_source,
                    &params.rate_at,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_purchase(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.create_with_items"))]
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
                    (id, user_id, wallet_id, category_id, merchant_id, store, description, notes,
                     purchase_date, currency, exchange_rate_scaled, rate_source, rate_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                   RETURNING *"#,
                &[
                    &purchase_id,
                    &params.user_id,
                    &params.wallet_id,
                    &params.category_id,
                    &params.merchant_id,
                    &params.store,
                    &params.description,
                    &params.notes,
                    &purchase_date,
                    &params.currency,
                    &params.exchange_rate_scaled,
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

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.get"))]
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

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.list"))]
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
                     AND ($6::uuid IS NULL OR merchant_id = $6)
                     AND ($7::text IS NULL OR store ILIKE '%' || $7 || '%')
                   ORDER BY purchase_date DESC, id DESC
                   LIMIT $8 OFFSET $9"#,
                &[
                    &user_id,
                    &start_date,
                    &filters.end_date,
                    &filters.wallet_id,
                    &filters.category_id,
                    &filters.merchant_id,
                    &filters.store,
                    &limit,
                    &offset,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_purchase).collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.update"))]
    async fn update(
        &self,
        purchase_id: Uuid,
        user_id: Uuid,
        current_version: i32,
        purchase_update: PurchaseUpdates,
        changes: Option<PurchaseChanges>,
    ) -> Result<Purchase> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let row = tran
            .query_opt(
                r#"UPDATE purchase SET
                    store       = COALESCE($1, store),
                    description = COALESCE($2, description),
                    notes       = COALESCE($3, notes),
                    category_id = COALESCE($4, category_id),
                    merchant_id = COALESCE($5, merchant_id),
                    version     = version + 1,
                    updated_at  = NOW()
                   WHERE id = $6 AND user_id = $7 AND version = $8 AND deleted_at IS NULL
                   RETURNING *"#,
                &[
                    &purchase_update.store,
                    &purchase_update.description,
                    &purchase_update.notes,
                    &purchase_update.category_id,
                    &purchase_update.merchant_id,
                    &purchase_id,
                    &user_id,
                    &current_version,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        let row = match row {
            Some(r) => r,
            None => return Err(miss_reason(&tran, purchase_id, user_id).await.into()),
        };

        let purchase = row_to_purchase(&row);

        if let Some(changes) = changes {
            insert_history(&tran, purchase_id, purchase.version, &changes).await?;
        }

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok(purchase)
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.delete"))]
    async fn delete(&self, id: Uuid, user_id: Uuid, changes: PurchaseChanges) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let row = tran
            .query_opt(
                "UPDATE purchase SET deleted_at = NOW(), version = version + 1 WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL RETURNING version",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        match row {
            Some(r) => {
                let version: i32 = r.get("version");
                insert_history(&tran, id, version, &changes).await?;
                tran.commit().await.map_err(RepositoryError::DbError)?;
                Ok(())
            }
            None => Err(RepositoryError::NotFound.into()),
        }
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.add_items"))]
    async fn add_items(
        &self,
        purchase_id: Uuid,
        user_id: Uuid,
        _current_version: i32,
        items: Vec<ItemInput>,
    ) -> Result<(Purchase, Vec<Item>)> {
        // Nothing to append: no-op (no version bump, no history row).
        if items.is_empty() {
            let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
            let row = conn
                .query_opt(
                    "SELECT * FROM purchase WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                    &[&purchase_id, &user_id],
                )
                .await
                .map_err(RepositoryError::DbError)?
                .ok_or(RepositoryError::NotFound)?;
            return Ok((row_to_purchase(&row), Vec::new()));
        }

        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let purchase = bump_purchase_version(&tran, purchase_id, user_id, _current_version).await?;

        let mut created = Vec::with_capacity(items.len());
        let base_seq = next_item_sequence(&tran, purchase_id).await?;
        let mut snapshots = Vec::with_capacity(items.len());

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
            let item = row_to_item(&item_row);
            snapshots.push(ItemSnapshot::from(&item));
            created.push(item);
        }

        let changes = PurchaseChanges::ItemAdded { items: snapshots };
        insert_history(&tran, purchase_id, purchase.version, &changes).await?;

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok((purchase, created))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.update_item"))]
    async fn update_item(
        &self,
        purchase_id: Uuid,
        user_id: Uuid,
        current_version: i32,
        sequence: i32,
        item: ItemUpdate,
        changes: PurchaseChanges,
    ) -> Result<Purchase> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let purchase = bump_purchase_version(&tran, purchase_id, user_id, current_version).await?;

        let row = tran
            .query_opt(
                r#"UPDATE item SET
                    name       = COALESCE($1, name),
                    unit_price = COALESCE($2, unit_price),
                    quantity   = COALESCE($3, quantity),
                    item_type  = COALESCE($4, item_type)
                   WHERE purchase_id = $5 AND item_sequence = $6
                   RETURNING item_sequence, purchase_id, name, unit_price::bigint AS unit_price, quantity, item_type"#,
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

        let _ = row_to_item(&row);
        insert_history(&tran, purchase_id, purchase.version, &changes).await?;

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok(purchase)
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "purchase.remove_item"))]
    async fn remove_item(
        &self,
        purchase_id: Uuid,
        user_id: Uuid,
        current_version: i32,
        sequence: i32,
        changes: PurchaseChanges,
    ) -> Result<Purchase> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let purchase = bump_purchase_version(&tran, purchase_id, user_id, current_version).await?;

        let n = tran
            .execute(
                "DELETE FROM item WHERE purchase_id = $1 AND item_sequence = $2",
                &[&purchase_id, &sequence],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        if n == 0 {
            return Err(RepositoryError::NotFound.into());
        }

        insert_history(&tran, purchase_id, purchase.version, &changes).await?;

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok(purchase)
    }
}
