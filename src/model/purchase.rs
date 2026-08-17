use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Purchase {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub wallet_id: uuid::Uuid,
    pub category_id: i32,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub version: i32,
    pub currency: String,
    pub exchange_rate: Option<f64>,
    pub rate_source: Option<String>,
    pub rate_at: Option<chrono::DateTime<chrono::Utc>>,
    pub purchase_date: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}
