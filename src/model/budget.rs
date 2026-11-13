use serde::{Deserialize, Serialize};

/// A Budget: a spending limit for one Category within one Wallet, over a
/// Budget period.
///
/// The template holds the recurring definition. Each occurrence is a Budget
/// period, holding the limit that actually applies during it (ADR-0009).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub wallet_id: uuid::Uuid,
    /// `None` means the limit covers the whole Wallet rather than one
    /// category.
    pub category_id: Option<i32>,
    pub name: String,
    /// The limit for one period, in minor units of the Wallet's currency.
    pub amount_minor: i64,
    pub cadence: BudgetCadence,
    /// The day of the month the period resets on: 1 for a calendar month, 15
    /// for someone paid on the fifteenth, -1 for the last day of the month.
    pub reset_day: i16,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    /// Whether unspent limit carries into the next period.
    pub rollover_unspent: bool,
    /// Whether an overspend carries into the next period.
    pub carry_overspend: bool,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// How often a budget recurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetCadence {
    Weekly,
    Monthly,
    Yearly,
}

impl BudgetCadence {
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetCadence::Weekly => "weekly",
            BudgetCadence::Monthly => "monthly",
            BudgetCadence::Yearly => "yearly",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "weekly" => Some(BudgetCadence::Weekly),
            "monthly" => Some(BudgetCadence::Monthly),
            "yearly" => Some(BudgetCadence::Yearly),
            _ => None,
        }
    }
}

/// One materialised occurrence of a Budget template.
///
/// The limit it holds is the *effective* limit: the template's amount, lowered
/// by a carried overspend, or replaced by a user edit. Nothing raises it
/// automatically (ADR-0010).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPeriod {
    pub id: uuid::Uuid,
    pub budget_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    /// The effective limit. May be zero or negative when a carried overspend
    /// exceeds the template's amount.
    pub amount_minor: i64,
    /// The limit the template specifies, kept so an edited period can be
    /// distinguished from a carried one.
    pub template_minor: i64,
    /// Money left over in this period. Reported, never added to the next
    /// period's limit (ADR-0010).
    pub unspent_minor: i64,
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// How a period is going, with the numbers a user needs to see it.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetStatus {
    pub period: BudgetPeriod,
    /// The currency the limit is in: the Wallet's.
    pub currency: String,
    /// What has been spent in this period, in the budget's category and wallet.
    pub spent_minor: i64,
    /// The limit less the spend. Negative when over.
    pub remaining_minor: i64,
    /// Whether the period has gone over its limit.
    pub over_budget: bool,
}
