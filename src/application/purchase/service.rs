use tracing::instrument;

use crate::application::{
    error::ApplicationError,
    inbound::repositories::{ItemRepository, PurchaseFilters, PurchaseRepository, PurchaseUpdates},
};
use crate::model::purchase::Purchase;

use super::commands::{CreatePurchaseCommand, UpdatePurchaseCommand};

pub struct PurchaseService<P, I>
where
    P: PurchaseRepository,
    I: ItemRepository,
{
    purchases: P,
    items: I,
}

impl<P, I> PurchaseService<P, I>
where
    P: PurchaseRepository,
    I: ItemRepository,
{
    pub fn new(purchases: P, items: I) -> Self {
        Self { purchases, items }
    }

    #[instrument(skip(self), fields(user_id = %cmd.user_id, wallet_id = %cmd.wallet_id))]
    pub async fn create(&self, cmd: CreatePurchaseCommand) -> Result<Purchase, ApplicationError> {
        let purchase = self
            .purchases
            .create(
                cmd.wallet_id,
                cmd.store,
                cmd.notes,
                cmd.user_id,
                cmd.category_id,
                cmd.purchase_date,
            )
            .await?;

        if !cmd.items.is_empty() {
            self.items.bulk_create(purchase.id, cmd.items).await?;
        }

        Ok(purchase)
    }

    #[instrument(skip(self), fields(purchase_id = %id, user_id = %user_id))]
    pub async fn get(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<Purchase, ApplicationError> {
        let purchase = self.purchases.get(id, user_id).await?;
        Ok(purchase)
    }

    #[instrument(skip(self), fields(user_id = %user_id))]
    pub async fn list(
        &self,
        user_id: uuid::Uuid,
        filters: PurchaseFilters,
    ) -> Result<Vec<Purchase>, ApplicationError> {
        let mut f = filters;
        if f.limit.is_none() {
            f.limit = Some(50);
        }
        if f.offset.is_none() {
            f.offset = Some(0);
        }
        if f.start_date.is_none() {
            f.start_date = Some(chrono::Utc::now() - chrono::Duration::days(30));
        }

        self.purchases.list(user_id, f).await
    }

    #[instrument(skip(self), fields(purchase_id = %cmd.id, user_id = %cmd.user_id))]
    pub async fn update(&self, cmd: UpdatePurchaseCommand) -> Result<Purchase, ApplicationError> {
        let updates = PurchaseUpdates {
            id: cmd.id,
            store: cmd.store,
            description: cmd.description,
            notes: cmd.notes,
            category_id: cmd.category_id,
        };

        self.purchases
            .update(cmd.id, cmd.user_id, cmd.current_version, updates)
            .await
    }

    #[instrument(skip(self), fields(purchase_id = %id, user_id = %user_id))]
    pub async fn delete(
        &self,
        id: uuid::Uuid,
        user_id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.purchases.delete(id, user_id).await
    }
}
