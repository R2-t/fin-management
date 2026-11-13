use serde::{Deserialize, Serialize};

/// An expectation that money will arrive on a cadence.
///
/// NOT money until an occurrence is received (ADR-0008). Booking it on the
/// scheduled date would invent money that may never arrive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomeSchedule {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub name: String,
    pub amount_minor: i64,
    pub currency: String,
    pub cadence: IncomeCadence,
    /// The first expected date. Later occurrences are derived from it.
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    pub active: bool,
    /// Where an occurrence is credited. One schedule may split across several
    /// wallets, as one intentional act (ADR-0003).
    pub allocations: Vec<IncomeAllocation>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One wallet's share of an Income schedule's amount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomeAllocation {
    pub id: uuid::Uuid,
    pub income_schedule_id: uuid::Uuid,
    pub wallet_id: uuid::Uuid,
    pub amount_minor: i64,
    pub currency: String,
}

/// How often income arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncomeCadence {
    Weekly,
    Biweekly,
    /// Twice a month, on the first and the fifteenth.
    Semimonthly,
    Monthly,
}

impl IncomeCadence {
    pub fn as_str(self) -> &'static str {
        match self {
            IncomeCadence::Weekly => "weekly",
            IncomeCadence::Biweekly => "biweekly",
            IncomeCadence::Semimonthly => "semimonthly",
            IncomeCadence::Monthly => "monthly",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "weekly" => Some(IncomeCadence::Weekly),
            "biweekly" => Some(IncomeCadence::Biweekly),
            "semimonthly" => Some(IncomeCadence::Semimonthly),
            "monthly" => Some(IncomeCadence::Monthly),
            _ => None,
        }
    }
}

/// A recorded occurrence: the point where an expectation becomes money
/// (ADR-0008). The ledger credit entries are written in the same transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomeReceipt {
    pub id: uuid::Uuid,
    pub income_schedule_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub amount_minor: i64,
    pub currency: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
