use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseHistory {
    pub id: uuid::Uuid,
    pub purchase_id: uuid::Uuid,
    pub version: i32,
    pub changes: serde_json::Value,
    pub changed_at: chrono::DateTime<chrono::Utc>,
}
