use chrono::Datelike;
use serde::{Deserialize, Serialize};

/// A declared recurring commitment: a subscription the user expects to keep
/// paying.
///
/// Forecasts subtract these; the ledger is not written until the user records
/// the actual purchase. A rule is a *declaration*, not a fact about money.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecurringRule {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub wallet_id: uuid::Uuid,
    pub merchant_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    pub name: String,
    pub amount_minor: i64,
    pub currency: String,
    pub cadence: RecurrenceCadence,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    pub status: RecurringStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// How often a recurring rule fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceCadence {
    Weekly,
    Biweekly,
    /// Twice a month, on the first and the fifteenth.
    Semimonthly,
    Monthly,
    Yearly,
}

impl RecurrenceCadence {
    pub fn as_str(self) -> &'static str {
        match self {
            RecurrenceCadence::Weekly => "weekly",
            RecurrenceCadence::Biweekly => "biweekly",
            RecurrenceCadence::Semimonthly => "semimonthly",
            RecurrenceCadence::Monthly => "monthly",
            RecurrenceCadence::Yearly => "yearly",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "weekly" => Some(RecurrenceCadence::Weekly),
            "biweekly" => Some(RecurrenceCadence::Biweekly),
            "semimonthly" => Some(RecurrenceCadence::Semimonthly),
            "monthly" => Some(RecurrenceCadence::Monthly),
            "yearly" => Some(RecurrenceCadence::Yearly),
            _ => None,
        }
    }

    /// The dates this cadence fires on, within `[from, to]`.
    ///
    /// Month arithmetic clamps to the last day of the month, so a rule anchored
    /// on the 31st fires on the 28th in February rather than skipping the
    /// month. The anchor day is preserved across the clamping: a rule anchored
    /// on the 31st returns to the 31st in March, not to the 28th it was
    /// clamped to in February.
    pub fn occurrences(
        self,
        starts_on: chrono::NaiveDate,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> Vec<chrono::NaiveDate> {
        let mut dates = Vec::new();
        if to < starts_on {
            return dates;
        }
        // The anchor day and month are fixed; only the year advances. This is
        // what keeps a 31st-anchored rule from drifting to the 28th.
        let anchor_day = starts_on.day();
        let mut current = starts_on;
        loop {
            if current > to {
                break;
            }
            if current >= from {
                dates.push(current);
            }
            current = match self {
                RecurrenceCadence::Weekly => current + chrono::Duration::days(7),
                RecurrenceCadence::Biweekly => current + chrono::Duration::days(14),
                RecurrenceCadence::Monthly => add_one_month(current, anchor_day),
                RecurrenceCadence::Yearly => add_one_year(current, anchor_day),
                RecurrenceCadence::Semimonthly => {
                    // The 15th of the same month, then the 1st of the next.
                    let fifteenth = current.with_day(15).unwrap_or(current);
                    if fifteenth > current {
                        if fifteenth <= to {
                            dates.push(fifteenth);
                        }
                        current = first_of_next_month(current);
                    } else {
                        current = first_of_next_month(current);
                    }
                    continue;
                }
            };
        }
        dates
    }
}

fn first_of_next_month(date: chrono::NaiveDate) -> chrono::NaiveDate {
    let (y, m) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(date)
}

/// Adds one month, clamping the day to the last day of the target month.
///
/// The anchor day is passed separately so a rule that was clamped in a short
/// month returns to its original day in the next long one.
fn add_one_month(date: chrono::NaiveDate, anchor_day: u32) -> chrono::NaiveDate {
    let (y, m) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    let last_day = days_in_month(y, m);
    let day = anchor_day.min(last_day);
    chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap_or(date)
}

fn add_one_year(date: chrono::NaiveDate, anchor_day: u32) -> chrono::NaiveDate {
    let y = date.year() + 1;
    let last_day = days_in_month(y, date.month());
    let day = anchor_day.min(last_day);
    chrono::NaiveDate::from_ymd_opt(y, date.month(), day).unwrap_or(date)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Whether a recurring rule still applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurringStatus {
    Active,
    Cancelled,
    Ended,
}

impl RecurringStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RecurringStatus::Active => "active",
            RecurringStatus::Cancelled => "cancelled",
            RecurringStatus::Ended => "ended",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(RecurringStatus::Active),
            "cancelled" => Some(RecurringStatus::Cancelled),
            "ended" => Some(RecurringStatus::Ended),
            _ => None,
        }
    }
}

/// One dated movement of money expected from a recurring rule.
#[derive(Debug, Clone, Serialize)]
pub struct RecurringOccurrence {
    pub rule_id: uuid::Uuid,
    pub name: String,
    pub date: chrono::NaiveDate,
    pub amount_minor: i64,
    pub currency: String,
    pub wallet_id: uuid::Uuid,
}
