use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub item_sequence: i32,
    pub purchase_id: uuid::Uuid,
    pub name: String,
    pub unit_price: i64,
    pub quantity: i32,
    pub item_type: Option<String>,
}
