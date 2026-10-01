use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-user reporting preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: uuid::Uuid,
    /// IANA timezone name, e.g. `Europe/Madrid`.
    pub timezone: String,
    /// Default reporting currency (ISO-4217), overridable per wallet.
    pub base_currency: String,
    pub locale: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
