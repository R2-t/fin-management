use serde::{Deserialize, Serialize};

/// A generated forecast, versioned so it can be evaluated and compared.
///
/// The result and the assumptions it was made under are stored together, so a
/// stale forecast explains itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Forecast {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub wallet_id: uuid::Uuid,
    pub kind: ForecastKind,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub horizon_days: i64,
    /// The result.
    pub result: serde_json::Value,
    /// The assumptions the result was made under.
    pub assumptions: serde_json::Value,
    /// The algorithm that produced it, so a result can be reproduced or
    /// superseded.
    pub model_version: String,
}

/// What a forecast is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastKind {
    /// Will a budget go over its limit before the period ends?
    BudgetOverspend,
    /// How long does a wallet's money last?
    CashRunway,
}

impl ForecastKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ForecastKind::BudgetOverspend => "budget_overspend",
            ForecastKind::CashRunway => "cash_runway",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "budget_overspend" => Some(ForecastKind::BudgetOverspend),
            "cash_runway" => Some(ForecastKind::CashRunway),
            _ => None,
        }
    }
}
