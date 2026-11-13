use chrono::{Datelike, NaiveDate};

/// One day's movements in a projection.
#[derive(Debug, Clone, Default)]
pub struct DayMovements {
    /// The discretionary spending projected for the day.
    pub discretionary_minor: i64,
    /// The recurring charges due on the day.
    pub recurring_minor: i64,
    /// The scheduled income due on the day.
    pub income_minor: i64,
}

/// What a projection found.
#[derive(Debug, Clone)]
pub struct Projection {
    /// The balance the projection started from.
    pub starting_balance_minor: i64,
    /// The lowest balance reached.
    pub trough_minor: i64,
    /// The day the trough falls on.
    pub trough_date: NaiveDate,
    /// The day the balance returns above zero, if it does.
    pub recovery_date: Option<NaiveDate>,
    /// The balance at the end of the horizon.
    pub ending_balance_minor: i64,
}

/// Projects a balance forward day by day.
///
/// This is the pure core of the runway: it takes the starting balance, the
/// days' movements, and the day's spending profile, and returns what the
/// projection found. Everything the money rules decide lives here, so it can
/// be exercised directly by the property tests.
///
/// The rules, in one place:
///
///  - The trough is the lowest balance reached, and never above the starting
///    balance.
///  - The recovery date exists only if the balance went below zero AND came
///    back. A balance that never dips has no recovery date -- it never ran
///    out. One that dips and never comes back has none either.
///  - The ending balance is the starting balance plus everything that
///    happened: spending and charges out, income in.
pub fn project(
    starting_balance_minor: i64,
    today: NaiveDate,
    days: &[DayMovements],
    // The spending for each day, indexed by the day's weekday offset from
    // Monday. `None` means every day costs the same.
    weekday_profile: Option<&[i64]>,
    flat_daily_minor: i64,
) -> Projection {
    let mut current = starting_balance_minor;
    let mut trough = current;
    let mut trough_date = today;
    let mut went_negative = current < 0;
    let mut recovery_date = None;

    for (offset, day) in days.iter().enumerate() {
        let date = today + chrono::Duration::days(offset as i64 + 1);

        // The discretionary spending: from the profile when there is one,
        // otherwise the flat pace.
        let discretionary = match weekday_profile {
            Some(profile) => {
                let index = date.weekday().num_days_from_monday() as usize;
                profile.get(index).copied().unwrap_or(0)
            }
            None => flat_daily_minor,
        };

        current -= discretionary;
        current -= day.recurring_minor;
        current += day.income_minor;

        if current < trough {
            trough = current;
            trough_date = date;
        }
        if current < 0 {
            went_negative = true;
        }
        if went_negative && recovery_date.is_none() && current > 0 {
            recovery_date = Some(date);
        }
    }

    Projection {
        starting_balance_minor,
        trough_minor: trough,
        trough_date,
        recovery_date,
        ending_balance_minor: current,
    }
}
