//! Property tests for the projection.
//!
//! A property test states an invariant that must hold for EVERY input, and
//! checks it across many generated inputs. The unit tests check specific
//! cases; these check the rules themselves.
//!
//! The invariants are the money rules the rest of the app depends on:
//!
//!  - The trough is never above the starting balance.
//!  - The ending balance is the starting balance plus everything that
//!    happened: spending and charges out, income in.
//!  - A recovery date exists only if the balance went below zero AND came
//!    back.
//!  - A recovery date, when it exists, is within the horizon.
//!  - The projection never creates money: with no income, the balance only
//!    goes down.

use super::projection::*;
use chrono::{Datelike, NaiveDate};

/// A small deterministic generator, so a failure is reproducible.
///
/// The properties are checked across a spread of inputs rather than random
/// ones, so the same failure is always the same failure.
fn seeds() -> Vec<(i64, i64, Vec<DayMovements>)> {
    let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let mut cases = Vec::new();

    // A spread of starting balances, paces and movement patterns.
    let balances = [-100_000i64, -1, 0, 1, 500, 100_000];
    let paces = [0i64, 1, 100, 10_000];
    let patterns: [(i64, i64); 4] = [(0, 0), (0, 5000), (5000, 0), (1000, 2000)];

    for balance in balances {
        for pace in paces {
            for (recurring, income) in patterns {
                let days = (0..30)
                    .map(|i| DayMovements {
                        recurring_minor: if i % 7 == 0 { recurring } else { 0 },
                        income_minor: if i % 30 == 15 { income } else { 0 },
                    })
                    .collect();
                cases.push((balance, pace, days));
            }
        }
    }

    // The current date is only used for the dates in the result, so it does
    // not affect the invariants.
    let _ = today;
    cases
}

#[test]
fn the_trough_is_never_above_the_starting_balance() {
    for (balance, pace, days) in seeds() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let projection = project(balance, today, &days, None, pace);

        assert!(
            projection.trough_minor <= balance,
            "the trough {trough} is above the starting balance {balance} at pace {pace}",
            trough = projection.trough_minor
        );
    }
}

#[test]
fn the_ending_balance_is_the_starting_balance_plus_what_happened() {
    for (balance, pace, days) in seeds() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let projection = project(balance, today, &days, None, pace);

        let charges: i64 = days.iter().map(|d| d.recurring_minor).sum();
        let income: i64 = days.iter().map(|d| d.income_minor).sum();
        let expected = balance + income - charges - pace * days.len() as i64;

        assert_eq!(
            projection.ending_balance_minor, expected,
            "the ending balance must be the arithmetic, at pace {pace}"
        );
    }
}

#[test]
fn a_recovery_date_implies_the_balance_dipped() {
    for (balance, pace, days) in seeds() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let projection = project(balance, today, &days, None, pace);

        // Stated from the projection's own outputs rather than by re-running
        // its loop. A previous version of this test transcribed `project`
        // line for line, so it would have agreed with any bug the two copies
        // shared.
        //
        // One direction only, because the other does not hold: a balance can
        // dip below zero and stay there, so a negative trough does not imply
        // a recovery.
        if let Some(recovery) = projection.recovery_date {
            assert!(
                projection.trough_minor < 0,
                "a recovery on {recovery} can only follow a dip; the trough is {} at pace {pace}",
                projection.trough_minor
            );
        }
    }
}

#[test]
fn a_recovery_date_is_within_the_horizon() {
    for (balance, pace, days) in seeds() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let projection = project(balance, today, &days, None, pace);

        if let Some(recovery) = projection.recovery_date {
            assert!(
                recovery > today,
                "the recovery must be after the start"
            );
            assert!(
                recovery <= today + chrono::Duration::days(days.len() as i64),
                "the recovery must be within the horizon"
            );
        }
    }
}

#[test]
fn the_projection_never_creates_money_without_income() {
    // With no income at all, the balance can only go down. This is the rule
    // that keeps a forecast from inventing money.
    for (balance, pace, _) in seeds() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let days = vec![
            DayMovements {
                recurring_minor: 0,
                income_minor: 0,
            };
            10
        ];

        let projection = project(balance, today, &days, None, pace);

        assert!(
            projection.ending_balance_minor <= balance,
            "without income the balance must not rise, at pace {pace}"
        );
        assert!(
            projection.trough_minor <= projection.ending_balance_minor,
            "the trough is the lowest point, so it cannot be above the ending balance"
        );
    }
}

#[test]
fn a_weekday_profile_changes_the_projection_but_not_the_arithmetic() {
    let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let days: Vec<DayMovements> = (0..30).map(|_| DayMovements::default()).collect();

    // A profile that spends nothing at the weekend.
    let profile = vec![1000i64, 1000, 1000, 1000, 1000, 0, 0];
    let with_profile = project(100_000, today, &days, Some(&profile), 0);

    // The same total spend spread evenly.
    let flat = project(100_000, today, &days, None, profile.iter().sum::<i64>() / 7);

    assert_ne!(
        with_profile.ending_balance_minor, flat.ending_balance_minor,
        "a weekday profile must change the projection"
    );

    // Both are internally consistent: the ending balance is the arithmetic.
    let expected = 100_000 - profile.iter().sum::<i64>() * 4 - 1000 * 2;
    assert_eq!(with_profile.ending_balance_minor, expected);
}

#[test]
fn a_zero_pace_leaves_the_balance_alone() {
    let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let days: Vec<DayMovements> = (0..30).map(|_| DayMovements::default()).collect();

    let projection = project(50_000, today, &days, None, 0);

    assert_eq!(projection.ending_balance_minor, 50_000);
    assert_eq!(projection.trough_minor, 50_000);
    assert!(projection.recovery_date.is_none());
}

#[test]
fn a_profile_with_missing_weekdays_costs_nothing_on_them() {
    let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let days: Vec<DayMovements> = (0..30).map(|_| DayMovements::default()).collect();

    // A profile that only covers Monday.
    let profile = vec![1000i64];
    let projection = project(100_000, today, &days, Some(&profile), 0);

    // Only the Mondays cost anything: four of them in thirty days.
    let mondays = days
        .iter()
        .enumerate()
        .filter(|(offset, _)| {
            (today + chrono::Duration::days(*offset as i64 + 1))
                .weekday()
                .num_days_from_monday()
                == 0
        })
        .count();

    assert_eq!(projection.ending_balance_minor, 100_000 - 1000 * mondays as i64);
}
