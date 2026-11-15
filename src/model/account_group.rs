/// A named set of the user's wallets, for reporting.
///
/// Not a consolidation: the app never sums across currencies (ADR-0002), so a
/// group total is per currency. It is a filter over wallets, nothing more.
#[derive(Debug, Clone)]
pub struct AccountGroup {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}
