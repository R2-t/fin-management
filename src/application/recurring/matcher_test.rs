//! Unit tests for the recurring matcher.
//!
//! The matcher decides whether a recorded purchase is the payment of a
//! declared recurring rule. A false positive removes spending from the burn
//! rate, which makes the forecast optimistic; a false negative leaves it in
//! and charges the subscription twice. The tests pin the conservative
//! direction.

use super::matcher::*;
use chrono::NaiveDate;
use crate::model::recurring::{RecurringRule, RecurringStatus, RecurrenceCadence};

fn rule(
    id: uuid::Uuid,
    merchant_id: Option<uuid::Uuid>,
    amount_minor: i64,
    cadence: RecurrenceCadence,
    starts_on: &str,
) -> RecurringRule {
    RecurringRule {
        id,
        user_id: uuid::Uuid::nil(),
        wallet_id: uuid::Uuid::nil(),
        merchant_id,
        category_id: None,
        name: "Subscription".into(),
        amount_minor,
        currency: "USD".into(),
        cadence,
        starts_on: date(starts_on),
        ends_on: None,
        status: RecurringStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        deleted_at: None,
    }
}

fn candidate(
    id: uuid::Uuid,
    merchant_id: Option<uuid::Uuid>,
    amount_minor: i64,
    on: &str,
) -> RecurringCandidate {
    RecurringCandidate {
        purchase_id: id,
        merchant_id,
        category_id: None,
        amount_minor,
        currency: "USD".into(),
        date: date(on),
    }
}

/// Parses "YYYY-MM-DD".
fn date(s: &str) -> NaiveDate {
    let mut parts = s.split('-');
    let year: i32 = parts.next().unwrap().parse().unwrap();
    let month: u32 = parts.next().unwrap().parse().unwrap();
    let day: u32 = parts.next().unwrap().parse().unwrap();
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

#[test]
fn a_purchase_on_the_day_and_amount_matches_exactly() {
    let merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(merchant), 1500, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].purchase_id, purchase);
    assert_eq!(matches[0].reason, MatchReason::Exact);
}

#[test]
fn a_purchase_a_few_days_off_still_matches() {
    let merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    // Three days late, which is the edge of the window.
    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(merchant), 1500, "2026-03-18")],
        &date("2026-03-15"),
    );

    assert_eq!(
        matches.len(),
        1,
        "a charge a few days late is still that charge"
    );
}

#[test]
fn a_purchase_outside_the_date_window_does_not_match() {
    let merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    // Four days late, one past the window.
    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(merchant), 1500, "2026-03-19")],
        &date("2026-03-15"),
    );

    assert!(
        matches.is_empty(),
        "a charge four days late is not confidently this charge"
    );
}

#[test]
fn a_purchase_at_a_different_merchant_does_not_match() {
    let rule_merchant = uuid::Uuid::new_v4();
    let other_merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(rule_merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    // The same amount, at a different merchant: two subscriptions can cost
    // the same, and matching one for the other would remove the wrong
    // spending from the burn rate.
    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(other_merchant), 1500, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert!(
        matches.is_empty(),
        "same amount at another merchant is not a match"
    );
}

#[test]
fn a_purchase_at_a_different_amount_does_not_match() {
    let merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    // More than 5% over.
    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(merchant), 2000, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert!(
        matches.is_empty(),
        "a different amount is not this subscription"
    );
}

#[test]
fn a_small_amount_difference_still_matches_as_drifted() {
    let merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    // 15% over: outside the exact band, inside the drift band.
    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(merchant), 1725, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].reason,
        MatchReason::AmountDrifted,
        "a price change is still the subscription"
    );
}

#[test]
fn a_rule_without_a_merchant_matches_on_amount_and_date_alone() {
    let rule = rule(
        uuid::Uuid::new_v4(),
        None,
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(uuid::Uuid::new_v4()), 1500, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert_eq!(
        matches.len(),
        1,
        "no merchant declared means any merchant may do"
    );
}

#[test]
fn one_purchase_is_matched_to_at_most_one_rule() {
    let merchant = uuid::Uuid::new_v4();
    let first = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let second = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    let matches = match_purchases(
        &[first, second],
        &[candidate(purchase, Some(merchant), 1500, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert_eq!(
        matches.len(),
        1,
        "a purchase cannot be charged as two subscriptions at once"
    );
}

#[test]
fn an_inactive_rule_matches_nothing() {
    let merchant = uuid::Uuid::new_v4();
    let mut cancelled = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    cancelled.status = RecurringStatus::Cancelled;
    let purchase = uuid::Uuid::new_v4();

    let matches = match_purchases(
        &[cancelled],
        &[candidate(purchase, Some(merchant), 1500, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert!(
        matches.is_empty(),
        "a cancelled rule is not expected to be paid"
    );
}

#[test]
fn a_purchase_in_another_currency_does_not_match() {
    let merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    let mut euro = candidate(purchase, Some(merchant), 1500, "2026-03-15");
    euro.currency = "EUR".into();

    let matches = match_purchases(&[rule], &[euro], &date("2026-03-15"));

    assert!(
        matches.is_empty(),
        "amounts in different currencies are not equal"
    );
}

#[test]
fn a_monthly_cadence_fires_on_the_same_day_each_month() {
    let dates = RecurrenceCadence::Monthly.occurrences(
        date("2026-01-15"),
        date("2026-01-01"),
        date("2026-04-30"),
    );

    assert_eq!(
        dates,
        vec![
            date("2026-01-15"),
            date("2026-02-15"),
            date("2026-03-15"),
            date("2026-04-15")
        ]
    );
}

#[test]
fn a_cadence_anchored_on_the_31st_clamps_to_short_months() {
    // A rule anchored on the 31st must still fire in February, on the 28th,
    // rather than skipping the month.
    let dates = RecurrenceCadence::Monthly.occurrences(
        date("2026-01-31"),
        date("2026-01-01"),
        date("2026-04-30"),
    );

    assert_eq!(
        dates,
        vec![
            date("2026-01-31"),
            date("2026-02-28"),
            date("2026-03-31"),
            date("2026-04-30")
        ]
    );
}

#[test]
fn a_semimonthly_cadence_fires_twice_a_month() {
    let dates = RecurrenceCadence::Semimonthly.occurrences(
        date("2026-01-01"),
        date("2026-01-01"),
        date("2026-02-28"),
    );

    assert_eq!(
        dates,
        vec![
            date("2026-01-01"),
            date("2026-01-15"),
            date("2026-02-01"),
            date("2026-02-15")
        ]
    );
}

#[test]
fn a_weekly_cadence_fires_every_seven_days() {
    let dates = RecurrenceCadence::Weekly.occurrences(
        date("2026-01-01"),
        date("2026-01-01"),
        date("2026-01-29"),
    );

    assert_eq!(
        dates,
        vec![
            date("2026-01-01"),
            date("2026-01-08"),
            date("2026-01-15"),
            date("2026-01-22"),
            date("2026-01-29")
        ]
    );
}

#[test]
fn a_biweekly_cadence_fires_every_fourteen_days() {
    let dates = RecurrenceCadence::Biweekly.occurrences(
        date("2026-01-01"),
        date("2026-01-01"),
        date("2026-01-29"),
    );

    assert_eq!(
        dates,
        vec![date("2026-01-01"), date("2026-01-15"), date("2026-01-29")]
    );
}

#[test]
fn occurrences_before_the_anchor_are_not_produced() {
    let dates = RecurrenceCadence::Monthly.occurrences(
        date("2026-06-01"),
        date("2026-01-01"),
        date("2026-12-31"),
    );

    assert_eq!(
        dates.first().unwrap(),
        &date("2026-06-01"),
        "a rule does not fire before it starts"
    );
    assert_eq!(dates.len(), 7);
}

#[test]
fn occurrences_outside_the_range_are_excluded() {
    let dates = RecurrenceCadence::Monthly.occurrences(
        date("2026-01-15"),
        date("2026-02-01"),
        date("2026-03-31"),
    );

    assert_eq!(dates, vec![date("2026-02-15"), date("2026-03-15")]);
}

#[test]
fn a_price_change_beyond_the_drift_band_does_not_match() {
    let merchant = uuid::Uuid::new_v4();
    let rule = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    // 50% over: beyond even the drift band, so a different purchase.
    let matches = match_purchases(
        &[rule],
        &[candidate(purchase, Some(merchant), 2250, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert!(matches.is_empty(), "a very different amount is not the subscription");
}

#[test]
fn a_purchase_with_no_merchant_does_not_match_a_rule_that_names_one() {
    let merchant = uuid::Uuid::new_v4();
    let subscription = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    let purchase = uuid::Uuid::new_v4();

    let matches = match_purchases(
        &[subscription],
        &[candidate(purchase, None, 1500, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert!(
        matches.is_empty(),
        "a purchase with no merchant cannot be confirmed as this subscription"
    );
}

#[test]
fn a_rule_that_has_ended_does_not_match_a_later_purchase() {
    let merchant = uuid::Uuid::new_v4();
    let mut ended = rule(
        uuid::Uuid::new_v4(),
        Some(merchant),
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    ended.ends_on = Some(date("2026-02-15"));
    let purchase = uuid::Uuid::new_v4();

    // March's charge cannot be the payment of a rule that ended in February.
    let matches = match_purchases(
        &[ended],
        &[candidate(purchase, Some(merchant), 1500, "2026-03-15")],
        &date("2026-03-15"),
    );

    assert!(
        matches.is_empty(),
        "a rule that has ended is not expected to be paid again"
    );
}

#[test]
fn occurrences_stop_at_the_rules_end_date() {
    let mut ended = rule(
        uuid::Uuid::new_v4(),
        None,
        1500,
        RecurrenceCadence::Monthly,
        "2026-01-15",
    );
    ended.ends_on = Some(date("2026-02-28"));

    let dates = occurrences_in_range(&ended, date("2026-01-01"), date("2026-06-30"));

    assert_eq!(
        dates,
        vec![date("2026-01-15"), date("2026-02-15")],
        "nothing after the end date is forecast"
    );
}
