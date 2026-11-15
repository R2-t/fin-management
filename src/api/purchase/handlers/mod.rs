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
    pub category_id: Option<i32>,
    pub classification_status: String,
    /// A category the system proposes but did not apply (D84). Only the
    /// merchant-history rule writes; the cross-user model proposes, and the
    /// client pre-selects this so confirming costs a glance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_category_id: Option<i32>,
    pub merchant_id: Option<Uuid>,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub version: i32,
    pub currency: String,
    /// Decimal places for `currency`; `unit_price` and totals are minor units.
    pub currency_minor_unit: u8,
    pub exchange_rate_scaled: Option<i64>,
    pub rate_source: Option<String>,
    pub rate_at: Option<chrono::DateTime<chrono::Utc>>,
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
            classification_status: p.classification_status,
            proposed_category_id: None,
            merchant_id: p.merchant_id,
            store: p.store,
            description: p.description,
            notes: p.notes,
            version: p.version,
            currency_minor_unit: crate::application::currency::minor_unit(&p.currency),
            currency: p.currency,
            exchange_rate_scaled: p.exchange_rate_scaled,
            rate_source: p.rate_source,
            rate_at: p.rate_at,
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
    pub unit_price: i64,
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
