//! Unit tests for the burn rate's arithmetic.
//!
//! The parts that matter are the ones that decide whether a forecast is
//! optimistic or pessimistic: the recurring payments must be removed from the
//! pace, and the weekday profile must not be skewed by one large purchase.

use super::service::*;
use chrono::NaiveDate;

fn day(date: &str, discretionary: i64) -> BurnDay {
    let mut parts = date.split('-');
    let year: i32 = parts.next().unwrap().parse().unwrap();
    let month: u32 = parts.next().unwrap().parse().unwrap();
    let day: u32 = parts.next().unwrap().parse().unwrap();
    BurnDay {
        day: NaiveDate::from_ymd_opt(year, month, day).unwrap(),
        total_minor: discretionary,
        discretionary_minor: discretionary,
    }
}

/// 2026-01-05 is a Monday, so the week runs 05..11.
#[test]
fn the_weekday_profile_is_monday_first() {
    // One Monday, one Tuesday.
    let days = vec![day("2026-01-05", 100), day("2026-01-06", 200)];

    let profile = weekday_medians(&days);

    assert_eq!(profile.len(), 7);
    assert_eq!(profile[0], 100, "Monday");
    assert_eq!(profile[1], 200, "Tuesday");
    assert_eq!(profile[2], 0, "Wednesday, with no history");
}

#[test]
fn the_profile_uses_the_median_not_the_mean() {
    // Four Mondays: 10, 10, 10 and 1000. The mean would be 257.5; the median
    // is 10, because one large purchase should not define the weekday's pace.
    let days = vec![
        day("2026-01-05", 10),
        day("2026-01-12", 10),
        day("2026-01-19", 10),
        day("2026-01-26", 1000),
    ];

    let profile = weekday_medians(&days);

    assert_eq!(profile[0], 10, "the median ignores the outlier");
}

#[test]
fn the_median_of_an_even_count_is_the_lower_middle_value() {
    // Two Mondays: 10 and 20. The lower middle is 10.
    let days = vec![day("2026-01-05", 10), day("2026-01-12", 20)];

    let profile = weekday_medians(&days);

    assert_eq!(profile[0], 10);
}

#[test]
fn a_weekday_with_no_history_contributes_nothing() {
    let days = vec![day("2026-01-05", 100)];

    let profile = weekday_medians(&days);

    assert_eq!(profile[0], 100);
    assert_eq!(profile[1..].iter().sum::<i64>(), 0);
}

#[test]
fn the_daily_pace_of_a_profile_is_the_average_of_the_medians() {
    // Monday 100, Tuesday 200, everything else 0. The profile averages to 300
    // over the seven days.
    let days = vec![day("2026-01-05", 100), day("2026-01-06", 200)];

    let profile = weekday_medians(&days);
    let daily = profile.iter().sum::<i64>() / 7;

    assert_eq!(daily, 42, "300 over the seven days");
}
