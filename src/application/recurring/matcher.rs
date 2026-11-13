use chrono::NaiveDate;

use crate::model::recurring::RecurringRule;

/// How closely a purchase must match a recurring rule to count as that
/// rule's payment.
///
/// The thresholds are deliberately conservative: a false positive removes
/// spending from the burn rate (making the forecast optimistic), and a false
/// negative leaves it in (making the forecast pessimistic) *and* charges the
/// subscription twice. Under-matching is the safer error, so the amount
/// tolerance is tight and the window is narrow.
pub mod thresholds {
    /// How far a purchase's amount may differ from the rule's amount.
    ///
    /// A subscription's price rarely varies; a 5% band absorbs small tax or
    /// tip differences without admitting a different purchase.
    pub const AMOUNT_TOLERANCE_NUM: i64 = 5;
    pub const AMOUNT_TOLERANCE_DEN: i64 = 100;

    /// The wider band within which a price change is still the same
    /// subscription.
    ///
    /// A price change is a modest move; a purchase at a very different amount
    /// is a different purchase even at the same merchant. Without this second
    /// band, buying lunch at the same shop as the subscription would match it.
    pub const AMOUNT_DRIFT_NUM: i64 = 25;
    pub const AMOUNT_DRIFT_DEN: i64 = 100;

    /// How many days either side of the expected date a purchase may fall.
    ///
    /// A charge that lands a few days early or late is still that charge.
    pub const DATE_WINDOW_DAYS: i64 = 3;
}

/// A purchase considered against the recurring rules.
#[derive(Debug, Clone)]
pub struct RecurringCandidate {
    pub purchase_id: uuid::Uuid,
    pub merchant_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    pub amount_minor: i64,
    pub currency: String,
    pub date: NaiveDate,
}

/// What matched, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchReason {
    /// The purchase matched on merchant, amount and date.
    Exact,
    /// The purchase matched on merchant and date, but the amount differed by
    /// a plausible price change.
    AmountDrifted,
}

impl MatchReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            MatchReason::Exact => "exact",
            MatchReason::AmountDrifted => "amount_drifted",
        }
    }
}

/// A purchase matched to a recurring rule.
#[derive(Debug, Clone)]
pub struct RecurringMatch {
    pub rule_id: uuid::Uuid,
    pub purchase_id: uuid::Uuid,
    pub date: NaiveDate,
    pub reason: MatchReason,
}

/// Matches recorded purchases against declared recurring rules.
///
/// This is the shared piece the burn rate needs now and Phase 4's
/// unintended-recurring detection will need later. It answers one question:
/// **is this purchase the payment of that recurring rule?**
///
/// A purchase matches a rule when it falls within the date window of one of
/// the rule's occurrences, at the rule's amount (within tolerance), for the
/// rule's merchant. A purchase matches at most one rule: the best match wins,
/// so a purchase cannot be charged as two subscriptions at once.
pub fn match_purchases(
    rules: &[RecurringRule],
    candidates: &[RecurringCandidate],
    window: &NaiveDate,
) -> Vec<RecurringMatch> {
    let mut matches = Vec::new();
    // A purchase is matched at most once.
    let mut claimed: Vec<uuid::Uuid> = Vec::new();

    for rule in rules {
        // Only a live rule can have payments.
        if rule.status != crate::model::recurring::RecurringStatus::Active {
            continue;
        }

        let occurrences = occurrences_in_range(
            rule,
            *window - chrono::Duration::days(thresholds::DATE_WINDOW_DAYS),
            *window + chrono::Duration::days(thresholds::DATE_WINDOW_DAYS),
        );

        for occurrence in occurrences {
            // The best candidate for this occurrence, if any.
            let mut best: Option<(RecurringMatch, u8, i64)> = None;

            for candidate in candidates {
                if claimed.contains(&candidate.purchase_id) {
                    continue;
                }
                if candidate.currency != rule.currency {
                    continue;
                }
                let date_delta = (candidate.date - occurrence).num_days();
                if date_delta.abs() > thresholds::DATE_WINDOW_DAYS {
                    continue;
                }

                let amount_ok = amount_matches(candidate.amount_minor, rule.amount_minor);
                let amount_drifted = amount_within_drift(candidate.amount_minor, rule.amount_minor);
                let merchant_ok = merchant_matches(candidate, rule);

                // The purchase must be recognisably this rule's payment: the
                // same merchant, and an amount that either matches or differs
                // by a plausible price change.
                //
                // A purchase at a DIFFERENT merchant is never a match, even at
                // exactly the rule's amount. Two subscriptions can cost the
                // same, and admitting one for the other would remove the wrong
                // spending from the burn rate.
                let reason = match (amount_ok, amount_drifted, merchant_ok) {
                    (true, _, true) => Some(MatchReason::Exact),
                    (false, true, true) => Some(MatchReason::AmountDrifted),
                    _ => None,
                };
                let reason = match reason {
                    Some(r) => r,
                    None => continue,
                };

                // Prefer an exact match, then the closest date.
                let quality = match reason {
                    MatchReason::Exact => 2,
                    _ => 1,
                };
                let better = match &best {
                    None => true,
                    Some((_, best_quality, best_delta)) => {
                        quality > *best_quality
                            || (quality == *best_quality && date_delta.abs() < *best_delta)
                    }
                };
                if better {
                    best = Some((
                        RecurringMatch {
                            rule_id: rule.id,
                            purchase_id: candidate.purchase_id,
                            date: occurrence,
                            reason,
                        },
                        quality,
                        date_delta.abs(),
                    ));
                }
            }

            if let Some((matched, _, _)) = best {
                claimed.push(matched.purchase_id);
                matches.push(matched);
            }
        }
    }

    matches
}

/// Whether a purchase's amount is within tolerance of the rule's amount.
///
/// Compared in integers: `|purchase - rule| * DEN <= rule * NUM`, with the
/// rule's amount as the base so the band scales with the subscription price.
fn amount_matches(purchase_minor: i64, rule_minor: i64) -> bool {
    amount_within(purchase_minor, rule_minor, thresholds::AMOUNT_TOLERANCE_NUM, thresholds::AMOUNT_TOLERANCE_DEN)
}

/// Whether a purchase's amount differs from the rule's by a plausible price
/// change rather than being a different purchase.
fn amount_within_drift(purchase_minor: i64, rule_minor: i64) -> bool {
    amount_within(purchase_minor, rule_minor, thresholds::AMOUNT_DRIFT_NUM, thresholds::AMOUNT_DRIFT_DEN)
}

fn amount_within(purchase_minor: i64, rule_minor: i64, num: i64, den: i64) -> bool {
    let diff = (purchase_minor as i128 - rule_minor as i128).abs();
    let band = rule_minor as i128 * num as i128 / den as i128;
    diff <= band
}

/// Whether the purchase is at the rule's merchant.
///
/// A rule with no merchant matches any purchase: the user has not said which
/// merchant it is, so the amount and the date are all there is to go on.
fn merchant_matches(candidate: &RecurringCandidate, rule: &RecurringRule) -> bool {
    match (rule.merchant_id, candidate.merchant_id) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(rule_merchant), Some(purchase_merchant)) => rule_merchant == purchase_merchant,
    }
}

/// The dates a rule fires on within a range, as dated movements.
///
/// Used by the forecast: each occurrence is a line item subtracted on its day.
/// A rule that has ended stops firing: its `ends_on` is the last day it can
/// charge, so projecting past it would invent a commitment the user cancelled.
pub fn occurrences_in_range(
    rule: &RecurringRule,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<NaiveDate> {
    let mut dates = rule.cadence.occurrences(rule.starts_on, from, to);
    if let Some(ends_on) = rule.ends_on {
        dates.retain(|d| *d <= ends_on);
    }
    dates
}
