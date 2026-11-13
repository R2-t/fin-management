use chrono::Datelike;
use serde::Serialize;
use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::AnalyticsRepository;
use crate::application::recurring::matcher::{self, RecurringCandidate};
use crate::model::recurring::RecurringRule;

/// The window the burn rate is measured over.
pub const BURN_WINDOW_DAYS: i64 = 30;

/// The minimum history the burn rate needs. Below this, no burn rate is
/// reported: inventing a pace is refused (D42).
pub const MIN_BURN_HISTORY_DAYS: i64 = 7;

/// How the daily pace was derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BurnSource {
    /// The weekday median profile: the median spend per weekday, scaled to the
    /// days in the range. Used once there is enough history for it (D43).
    WeekdayProfile,
    /// A flat daily average over the whole window. Used when there is not
    /// enough history for a profile.
    FlatAverage,
}

impl BurnSource {
    pub fn as_str(self) -> &'static str {
        match self {
            BurnSource::WeekdayProfile => "weekday_profile",
            BurnSource::FlatAverage => "flat_average",
        }
    }
}

/// One day's discretionary spend, and its weekday.
#[derive(Debug, Clone, Serialize)]
pub struct BurnDay {
    pub day: chrono::NaiveDate,
    /// Discretionary spend that day: the total minus the payments of the
    /// declared recurring rules.
    pub discretionary_minor: i64,
    pub total_minor: i64,
}

/// The spending pace a forecast assumes.
#[derive(Debug, Clone, Serialize)]
pub struct BurnRate {
    /// The pace per day, in minor units.
    pub daily_minor: i64,
    /// The days of history the pace was measured over.
    pub history_days: i64,
    /// How the pace was derived.
    pub source: BurnSource,
    /// The per-day amounts, oldest first.
    pub days: Vec<BurnDay>,
    /// The weekday median profile, when one was used: seven amounts, Monday
    /// first. `None` for a flat average.
    pub weekday_profile: Option<Vec<i64>>,
}

/// Reads the discretionary spending pace.
///
/// The pace excludes the recorded payments of the declared recurring rules.
/// A subscription that is both inside the trailing spend and declared as a
/// rule would otherwise be subtracted twice -- once inside the pace and once
/// as a dated movement -- which would make every forecast pessimistic by the
/// cost of every subscription the user has declared (D40).
pub struct BurnRateService<A>
where
    A: AnalyticsRepository,
{
    analytics: A,
}

impl<A> BurnRateService<A>
where
    A: AnalyticsRepository,
{
    pub fn new(analytics: A) -> Self {
        Self { analytics }
    }

    /// The pace over the trailing window ending today.
    ///
    /// `rules` are the user's declared recurring commitments; the recorded
    /// payments of those rules are removed from the spend.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn burn_rate(
        &self,
        user_id: uuid::Uuid,
        wallet_id: Option<uuid::Uuid>,
        rules: &[RecurringRule],
    ) -> Result<BurnRate, ApplicationError> {
        self.burn_rate_over(user_id, wallet_id, rules, BURN_WINDOW_DAYS).await
    }

    /// The pace over an explicit window, used by the forecast to measure over
    /// the same span it projects.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, window_days = %window_days))]
    pub async fn burn_rate_over(
        &self,
        user_id: uuid::Uuid,
        wallet_id: Option<uuid::Uuid>,
        rules: &[RecurringRule],
        window_days: i64,
    ) -> Result<BurnRate, ApplicationError> {
        let today = chrono::Utc::now().date_naive();

        // The window starts at the user's first purchase, not at
        // `today - window_days`: a user who started recording two weeks ago
        // has two weeks of history, and counting the days before that as
        // zero-spend days would understate their pace.
        let first = self
            .analytics
            .first_purchase_date(user_id, wallet_id)
            .await?;
        let history_start = match first {
            Some(first) => first.min(today),
            None => today,
        };
        let window_start = history_start.max(today - chrono::Duration::days(window_days - 1));
        let from = window_start;

        let range = crate::application::inbound::repositories::SpendRange {
            from: from.and_time(chrono::NaiveTime::MIN).and_utc(),
            to: (today + chrono::Duration::days(1))
                .and_time(chrono::NaiveTime::MIN)
                .and_utc(),
            wallet_id,
            category_id: None,
            currency: None,
            timezone: "UTC".to_string(),
        };

        // The daily spend, and the purchases behind it, so the recurring
        // payments can be removed.
        let series = self.analytics.daily_spend(user_id, &range, 0).await?;
        let spend = series
            .into_iter()
            .next()
            .ok_or_else(|| ApplicationError::Internal("no spend series returned".into()))?;

        let purchases = self
            .analytics
            .purchases_in_range(user_id, &range)
            .await?;

        // Which recorded purchases are the payments of a declared rule?
        let candidates: Vec<RecurringCandidate> = purchases
            .iter()
            .map(|p| RecurringCandidate {
                purchase_id: p.purchase_id,
                merchant_id: p.merchant_id,
                category_id: p.category_id,
                amount_minor: p.amount_minor,
                currency: p.currency.clone(),
                date: p.occurred_at.date_naive(),
            })
            .collect();

        let matched = matcher::match_purchases(rules, &candidates, &today);
        let recurring_totals = sum_by_day(&matched, &candidates);

        eprintln!("DEBUG from={:?} to={:?} first={:?} today={:?} points={} days={:?}", range.from, range.to, first, today, spend.points.len(), spend.points.iter().map(|p| (p.day, p.total_minor)).collect::<Vec<_>>());

        let days: Vec<BurnDay> = spend
            .points
            .iter()
            .map(|point| {
                let recurring = recurring_totals
                    .iter()
                    .find(|(day, _)| *day == point.day)
                    .map(|(_, total)| *total)
                    .unwrap_or(0);
                BurnDay {
                    day: point.day,
                    total_minor: point.total_minor,
                    discretionary_minor: point.total_minor - recurring,
                }
            })
            .collect();

        // The window is the days that have history, not the days asked for.
        // A user who started recording last week has a week of history, not 30
        // days of it.
        let history_days = days.len() as i64;
        if history_days < MIN_BURN_HISTORY_DAYS {
            return Ok(BurnRate {
                daily_minor: 0,
                history_days,
                source: BurnSource::FlatAverage,
                days,
                weekday_profile: None,
            });
        }

        // The weekday profile needs at least a couple of weeks for each
        // weekday to mean anything.
        let use_profile = history_days >= MIN_BURN_HISTORY_DAYS * 2;

        let (daily_minor, weekday_profile) = if use_profile {
            let profile = weekday_medians(&days);
            let projected = profile.iter().sum::<i64>() / 7;
            (projected, Some(profile))
        } else {
            let total: i64 = days.iter().map(|d| d.discretionary_minor).sum();
            (total / history_days, None)
        };

        Ok(BurnRate {
            daily_minor,
            history_days,
            source: if use_profile {
                BurnSource::WeekdayProfile
            } else {
                BurnSource::FlatAverage
            },
            days,
            weekday_profile,
        })
    }
}

/// Sums the matched purchases by the day they occurred.
fn sum_by_day(
    matched: &[matcher::RecurringMatch],
    candidates: &[RecurringCandidate],
) -> Vec<(chrono::NaiveDate, i64)> {
    let mut totals: Vec<(chrono::NaiveDate, i64)> = Vec::new();
    for m in matched {
        let candidate = match candidates.iter().find(|c| c.purchase_id == m.purchase_id) {
            Some(c) => c,
            None => continue,
        };
        match totals.iter_mut().find(|(day, _)| *day == candidate.date) {
            Some((_, total)) => *total += candidate.amount_minor,
            None => totals.push((candidate.date, candidate.amount_minor)),
        }
    }
    totals
}

/// The median discretionary spend per weekday, Monday first.
///
/// Visible to the module's tests.
///
/// The median rather than the mean: one large purchase should not define a
/// weekday's pace. The median of an even count is the lower of the two middle
/// values, so a weekday with few observations does not inflate the pace.
pub(super) fn weekday_medians(days: &[BurnDay]) -> Vec<i64> {
    // `weekday` is 1 = Monday .. 7 = Sunday.
    let mut buckets: Vec<Vec<i64>> = vec![Vec::new(); 7];
    for day in days {
        let index = (day.day.weekday().num_days_from_monday()) as usize;
        buckets[index].push(day.discretionary_minor);
    }

    buckets
        .into_iter()
        .map(|mut values| {
            if values.is_empty() {
                return 0;
            }
            values.sort_unstable();
            // The lower middle value: index (len - 1) / 2.
            values[(values.len() - 1) / 2]
        })
        .collect()
}
