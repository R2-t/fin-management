use crate::application::inbound::repositories::ItemInput;

#[derive(Debug)]
pub struct CreatePurchaseCommand {
    pub user_id: uuid::Uuid,
    pub wallet_id: uuid::Uuid,
    pub category_id: i32,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub currency: String,
    pub exchange_rate_override: Option<f64>,
    pub purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    pub items: Vec<ItemInput>,
}

#[derive(Debug)]
pub struct UpdatePurchaseCommand {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub current_version: i32,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub category_id: Option<i32>,
}
