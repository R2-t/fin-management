use chrono::{Datelike, NaiveDate};

use crate::model::budget::{Budget, BudgetCadence};

/// The period a budget covers on a given day, and the period before it.
///
/// A period is materialised lazily, when it is first asked about (ADR-0009).
/// This module computes which period a day falls in, without touching the
/// database.
pub struct PeriodBounds {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

/// The period containing `on`.
///
/// For a monthly budget the period starts on the reset day. A reset day of -1
/// means the last day of the month, so the period runs from the last day of
/// one month to the last day of the next.
pub fn period_containing(budget: &Budget, on: NaiveDate) -> PeriodBounds {
    match budget.cadence {
        BudgetCadence::Weekly => {
            // Weeks start on Monday.
            let start = on - chrono::Duration::days(on.weekday().num_days_from_monday() as i64);
            PeriodBounds {
                start,
                end: start + chrono::Duration::days(6),
            }
        }
        BudgetCadence::Monthly => {
            let start = monthly_period_start(budget.reset_day, on);
            let end = next_monthly_period_start(budget.reset_day, start) - chrono::Duration::days(1);
            PeriodBounds { start, end }
        }
        BudgetCadence::Yearly => {
            let start = NaiveDate::from_ymd_opt(on.year(), 1, 1).unwrap_or(on);
            PeriodBounds {
                start,
                end: NaiveDate::from_ymd_opt(on.year() + 1, 1, 1)
                    .unwrap_or(on)
                    - chrono::Duration::days(1),
            }
        }
    }
}

/// The start of the monthly period containing `on`.
///
/// The reset day is the day the period *starts*, so a reset day of 1 gives the
/// calendar month, and a reset day of 15 gives the 15th to the 14th.
///
/// A reset day of -1 means the period ENDS on the last day of the month, so
/// it runs from the first of one month to the last of that month. That is the
/// same as a reset day of 1 for the start, but it is expressed as an end, so
/// the two differ only in how a month-end payday reads.
fn monthly_period_start(reset_day: i16, on: NaiveDate) -> NaiveDate {
    let day = reset_day.clamp(1, 28) as u32;
    let this_month_start = NaiveDate::from_ymd_opt(on.year(), on.month(), day).unwrap_or(on);
    if on >= this_month_start {
        this_month_start
    } else {
        let (py, pm) = if on.month() == 1 {
            (on.year() - 1, 12)
        } else {
            (on.year(), on.month() - 1)
        };
        NaiveDate::from_ymd_opt(py, pm, day).unwrap_or(on)
    }
}

/// The start of the monthly period AFTER the one starting at `start`.
///
/// Computed by advancing one calendar month from the start's own day, which
/// avoids the ambiguity of probing with a day that may fall in the next period.
fn next_monthly_period_start(reset_day: i16, start: NaiveDate) -> NaiveDate {
    if reset_day == -1 {
        // The period ends on the last day of its month, so the next one starts
        // on the first of the next month.
        let (y, m) = if start.month() == 12 {
            (start.year() + 1, 1)
        } else {
            (start.year(), start.month() + 1)
        };
        return NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(start);
    }
    let day = reset_day.clamp(1, 28) as u32;
    let (y, m) = if start.month() == 12 {
        (start.year() + 1, 1)
    } else {
        (start.year(), start.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, day).unwrap_or(start)
}

/// The period before the one starting at `start`.
///
/// Used to compute a period's carry-in from the period that preceded it.
pub fn previous_period(budget: &Budget, start: NaiveDate) -> PeriodBounds {
    match budget.cadence {
        BudgetCadence::Weekly => PeriodBounds {
            start: start - chrono::Duration::days(7),
            end: start - chrono::Duration::days(1),
        },
        BudgetCadence::Monthly => {
            let end = start - chrono::Duration::days(1);
            PeriodBounds {
                start: monthly_period_start(budget.reset_day, end),
                end,
            }
        }
        BudgetCadence::Yearly => PeriodBounds {
            start: NaiveDate::from_ymd_opt(start.year() - 1, 1, 1).unwrap_or(start),
            end: start - chrono::Duration::days(1),
        },
    }
}

/// The limit a period's effective limit is, given the previous period's
/// outcome.
///
/// An overspend carries into the next period, lowering its limit (D30). The
/// carry is bounded below: a period whose limit would go below zero is
/// already overspent by more than the limit, and a negative limit says
/// exactly that.
///
/// Unspent money does NOT raise the next period's limit (ADR-0010). The user
/// may be deliberately saving it, and a limit that rose on its own would spend
/// that saving twice.
pub fn effective_limit(
    template_minor: i64,
    previous_spent_minor: Option<i64>,
    carry_overspend: bool,
) -> i64 {
    match previous_spent_minor {
        Some(spent) if carry_overspend => {
            let overspend = spent - template_minor;
            if overspend > 0 {
                template_minor - overspend
            } else {
                template_minor
            }
        }
        _ => template_minor,
    }
}

/// The money left over in a period, given its limit and its spend.
///
/// Only meaningful when the spend is under the limit. Reported, never carried
/// into the next period's limit.
pub fn unspent(limit_minor: i64, spent_minor: i64) -> i64 {
    if spent_minor < limit_minor {
        limit_minor - spent_minor
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(cadence: BudgetCadence, reset_day: i16) -> Budget {
        Budget {
            id: uuid::Uuid::nil(),
            user_id: uuid::Uuid::nil(),
            wallet_id: uuid::Uuid::nil(),
            category_id: None,
            name: "Test".into(),
            amount_minor: 500,
            cadence,
            reset_day,
            starts_on: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            ends_on: None,
            rollover_unspent: true,
            carry_overspend: true,
            active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        }
    }

    fn date(s: &str) -> NaiveDate {
        let mut parts = s.split('-');
        let year: i32 = parts.next().unwrap().parse().unwrap();
        let month: u32 = parts.next().unwrap().parse().unwrap();
        let day: u32 = parts.next().unwrap().parse().unwrap();
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn a_reset_day_of_one_gives_the_calendar_month() {
        let b = budget(BudgetCadence::Monthly, 1);
        let period = period_containing(&b, date("2026-03-15"));

        assert_eq!(period.start, date("2026-03-01"));
        assert_eq!(period.end, date("2026-03-31"));
    }

    #[test]
    fn a_reset_day_of_fifteen_gives_the_mid_month_period() {
        let b = budget(BudgetCadence::Monthly, 15);
        let period = period_containing(&b, date("2026-03-20"));

        assert_eq!(period.start, date("2026-03-15"));
        assert_eq!(period.end, date("2026-04-14"));
    }

    #[test]
    fn a_reset_day_of_minus_one_ends_the_period_on_the_last_day_of_the_month() {
        // -1 means the period ENDS on the last day of the month, so it runs
        // from the first of one month to the last of that month.
        let b = budget(BudgetCadence::Monthly, -1);
        let period = period_containing(&b, date("2026-03-20"));

        assert_eq!(period.start, date("2026-03-01"));
        assert_eq!(period.end, date("2026-03-31"));
    }

    #[test]
    fn the_period_of_the_first_day_of_a_month_is_its_own() {
        let b = budget(BudgetCadence::Monthly, 1);
        let period = period_containing(&b, date("2026-03-01"));

        assert_eq!(period.start, date("2026-03-01"));
        assert_eq!(period.end, date("2026-03-31"));
    }

    #[test]
    fn a_weekly_period_starts_on_monday() {
        let b = budget(BudgetCadence::Weekly, 1);
        // 2026-03-18 is a Wednesday.
        let period = period_containing(&b, date("2026-03-18"));

        assert_eq!(period.start, date("2026-03-16"));
        assert_eq!(period.end, date("2026-03-22"));
        assert_eq!(period.start.weekday().num_days_from_monday(), 0);
    }

    #[test]
    fn the_previous_monthly_period_ends_the_day_before() {
        let b = budget(BudgetCadence::Monthly, 15);
        let previous = previous_period(&b, date("2026-03-15"));

        assert_eq!(previous.end, date("2026-03-14"));
        assert_eq!(previous.start, date("2026-02-15"));
    }

    #[test]
    fn an_overspend_lowers_the_next_periods_limit() {
        // Spent 700 against a 500 limit: 200 over, so the next period is 300.
        assert_eq!(effective_limit(500, Some(700), true), 300);
    }

    #[test]
    fn spending_under_the_limit_leaves_the_limit_alone() {
        assert_eq!(effective_limit(500, Some(300), true), 500);
    }

    #[test]
    fn without_the_carry_flag_an_overspend_is_forgiven() {
        assert_eq!(effective_limit(500, Some(700), false), 500);
    }

    #[test]
    fn a_limit_can_go_negative_when_the_overspend_exceeds_it() {
        // Spent 700 against a 500 limit: 200 over, so the next period is -200.
        assert_eq!(effective_limit(200, Some(700), true), -300);
    }

    #[test]
    fn unspent_is_the_difference_when_under_the_limit() {
        assert_eq!(unspent(500, 300), 200);
    }

    #[test]
    fn unspent_is_zero_when_over_the_limit() {
        assert_eq!(unspent(500, 700), 0);
    }
}
