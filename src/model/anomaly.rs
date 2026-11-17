use serde::{Deserialize, Serialize};

/// A finding about records that look wrong.
///
/// It is *about* records and never changes them: dismissing is the only write,
/// and it touches only this row.
#[derive(Debug, Clone)]
pub struct Anomaly {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub kind: String,
    pub subject_type: String,
    pub subject_id: Option<uuid::Uuid>,
    pub title: String,
    pub detail: serde_json::Value,
    pub severity: String,
    pub detected_at: chrono::DateTime<chrono::Utc>,
    pub key: String,
    pub dismissed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub dismissed_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnomalyStatus {
    Open,
    Dismissed,
}

impl AnomalyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnomalySeverity {
    Notice,
    Warning,
}

impl AnomalySeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Warning => "warning",
        }
    }
}
