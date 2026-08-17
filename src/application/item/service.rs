use uuid;

use super::commands::{CreateItemCommand, UpdateItemCommand};
use crate::application::inbound::repositories::ItemInput;
use crate::application::inbound::repositories::ItemUpdate;
use crate::application::inbound::repositories::PurchaseHistoryRepository;
use crate::application::{error::ApplicationError, inbound::repositories::ItemRepository};
use crate::model::item::Item;

pub struct ItemService<I, H>
where
    I: ItemRepository,
    H: PurchaseHistoryRepository,
{
    items: I,
    #[allow(dead_code)]
    purchase_history: H,
}

impl<I: ItemRepository, H: PurchaseHistoryRepository> ItemService<I, H> {
    pub fn new(items: I, purchase_history: H) -> Self {
        Self {
            items,
            purchase_history,
        }
    }

    pub async fn bulk_create(
        &self,
        purchase_id: uuid::Uuid,
        cmd: Vec<CreateItemCommand>,
    ) -> Result<Vec<Item>, ApplicationError> {
        let item_inputs = cmd
            .into_iter()
            .map(|item| ItemInput {
                name: item.name,
                quantity: item.quantity,
                unit_price: item.unit_price,
                item_type: item.item_type,
            })
            .collect();

        let items = self.items.bulk_create(purchase_id, item_inputs).await?;

        Ok(items)
    }

    pub async fn list_by_purchase(
        &self,
        purchase_id: uuid::Uuid,
    ) -> Result<Vec<Item>, ApplicationError> {
        Ok(self.items.list_by_purchase(purchase_id).await?)
    }

    pub async fn update(
        &self,
        purchase_id: uuid::Uuid,
        sequence: i32,
        item: UpdateItemCommand,
    ) -> Result<Item, ApplicationError> {
        let updates = ItemUpdate {
            name: item.name,
            unit_price: item.unit_price,
            quantity: item.quantity,
            item_type: item.item_type,
        };

        Ok(self.items.update(purchase_id, sequence, updates).await?)
    }

    pub async fn delete(
        &self,
        purchase_id: uuid::Uuid,
        sequence: i32,
    ) -> Result<(), ApplicationError> {
        Ok(self.items.delete(purchase_id, sequence).await?)
    }
}
