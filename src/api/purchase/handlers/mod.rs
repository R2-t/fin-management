use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::purchase::Purchase;

pub mod create_purchase_with_items;
pub mod delete_purchase;
pub mod get_purchase_details;
pub mod list_purchases;
pub mod update_purchase;

#[derive(Debug, Clone, Serialize)]
pub struct PurchaseResponse {
    pub id: Uuid,
    pub user_id: Uuid,
    pub wallet_id: Uuid,
    pub category_id: i32,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub version: i32,
    pub purchase_date: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<Purchase> for PurchaseResponse {
    fn from(p: Purchase) -> Self {
        Self {
            id: p.id,
            user_id: p.user_id,
            wallet_id: p.wallet_id,
            category_id: p.category_id,
            store: p.store,
            description: p.description,
            notes: p.notes,
            version: p.version,
            purchase_date: p.purchase_date,
            created_at: p.created_at,
            updated_at: p.updated_at,
            deleted_at: p.deleted_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ItemDto {
    pub name: String,
    pub unit_price: f64,
    pub quantity: i32,
    pub item_type: Option<String>,
}

impl From<ItemDto> for crate::application::inbound::repositories::ItemInput {
    fn from(d: ItemDto) -> Self {
        Self {
            name: d.name,
            unit_price: d.unit_price,
            quantity: d.quantity,
            item_type: d.item_type,
        }
    }
}
