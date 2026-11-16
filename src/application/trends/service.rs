use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{
    AnalyticsRepository, DailyPoint, SeriesPoint, SpendRange, TrendBucket,
};

/// Growth of a bucket against the previous one, per currency. `delta_pct` is a
/// dimensionless ratio, the one place MONEY.md permits a float.
#[derive(Debug, Clone, Serialize)]
pub struct Growth {
    pub previous_total: i64,
    pub delta_minor: i64,
    /// `None` when the previous bucket was zero, where a percentage is undefined.
    pub delta_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendPoint {
    pub bucket_start: DateTime<Utc>,
    pub total_minor: i64,
    pub purchases: i64,
    pub unresolved: i64,
    /// Present only for month buckets; `None` for the first bucket of a series.
    pub growth: Option<Growth>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendSeries {
    pub currency: String,
    pub points: Vec<TrendPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendsView {
    /// When this answer was computed.
    pub as_of: DateTime<Utc>,
    pub bucket: TrendBucket,
    pub timezone: String,
    /// Age of the oldest pending projection work, so a caller can tell how
    /// fresh the figures are.
    pub projection_delay_seconds: Option<f64>,
    pub series: Vec<TrendSeries>,
}

/// A day's spend with its trailing-window average. The average is money, so it
/// is an integer rounded with the project's single rule, never a float.
#[derive(Debug, Clone, Serialize)]
pub struct RollingPoint {
    pub day: chrono::NaiveDate,
    pub total_minor: i64,
    pub avg_minor: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RollingSeries {
    pub currency: String,
    pub points: Vec<RollingPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RollingView {
    pub as_of: DateTime<Utc>,
    pub window_days: i32,
    pub timezone: String,
    pub projection_delay_seconds: Option<f64>,
    pub series: Vec<RollingSeries>,
}

/// Read model over the analytics projection. Contains no money arithmetic that
/// MONEY.md forbids: totals are per-currency integers produced by SQL, and the
/// only float in the response is a dimensionless growth percentage.
pub struct TrendsService<T>
where
    T: AnalyticsRepository,
{
    analytics: T,
}

impl<T> TrendsService<T>
where
    T: AnalyticsRepository,
{
    pub fn new(analytics: T) -> Self {
        Self { analytics }
    }

    /// Bucketed spend per currency, with month-over-month growth when the
    /// bucket is a month.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, bucket = ?query.bucket))]
    pub async fn trends(
        &self,
        user_id: uuid::Uuid,
        query: TrendsQuery,
    ) -> Result<TrendsView, ApplicationError> {
        let series = self
            .analytics
            .bucketed_spend(user_id, &query.range, query.bucket)
            .await?;

        let with_growth = query.bucket == TrendBucket::Month;
        let series = series
            .into_iter()
            .map(|s| TrendSeries {
                currency: s.currency,
                points: attach_growth(s.points, with_growth),
            })
            .collect();

        Ok(TrendsView {
            as_of: Utc::now(),
            bucket: query.bucket,
            timezone: query.range.timezone.clone(),
            projection_delay_seconds: self.projection_delay().await,
            series,
        })
    }

    /// Daily spend per currency with a trailing-window average.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, window_days = %query.window_days))]
    pub async fn rolling(
        &self,
        user_id: uuid::Uuid,
        query: RollingQuery,
    ) -> Result<RollingView, ApplicationError> {
        let series = self
            .analytics
            .daily_spend(user_id, &query.range, query.window_days)
            .await?;

        let series = series
            .into_iter()
            .map(|s| RollingSeries {
                currency: s.currency,
                points: rolling_average(&s.points, query.window_days),
            })
            .collect();

        Ok(RollingView {
            as_of: Utc::now(),
            window_days: query.window_days,
            timezone: query.range.timezone.clone(),
            projection_delay_seconds: self.projection_delay().await,
            series,
        })
    }

    async fn projection_delay(&self) -> Option<f64> {
        self.analytics.outbox_lag_seconds().await.ok().flatten()
    }
}

/// A bucket range to report on.
#[derive(Debug, Clone)]
pub struct TrendsQuery {
    pub range: SpendRange,
    pub bucket: TrendBucket,
}

/// A rolling-average window to report on.
#[derive(Debug, Clone)]
pub struct RollingQuery {
    pub range: SpendRange,
    pub window_days: i32,
}

/// Consecutive-bucket growth. The first bucket has no predecessor, so it carries
/// no growth figure.
fn attach_growth(points: Vec<SeriesPoint>, with_growth: bool) -> Vec<TrendPoint> {
    let mut previous: Option<i64> = None;
    points
        .into_iter()
        .map(|p| {
            let growth = if with_growth {
                previous.map(|previous_total| Growth {
                    previous_total,
                    delta_minor: p.total_minor - previous_total,
                    delta_pct: if previous_total == 0 {
                        None
                    } else {
                        Some((p.total_minor - previous_total) as f64 / previous_total as f64 * 100.0)
                    },
                })
            } else {
                None
            };
            previous = Some(p.total_minor);
            TrendPoint {
                bucket_start: p.bucket_start,
                total_minor: p.total_minor,
                purchases: p.purchases,
                unresolved: p.unresolved,
                growth,
            }
        })
        .collect()
}

/// Trailing-window average over the days available, rounded half away from zero.
/// Early days average a shorter window, which is what a trailing average means.
fn rolling_average(points: &[DailyPoint], window_days: i32) -> Vec<RollingPoint> {
    let window = window_days.max(1) as usize;
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let start = (i + 1).saturating_sub(window);
            let slice = &points[start..=i];
            let sum: i128 = slice.iter().map(|d| d.total_minor as i128).sum();
            let count = slice.len() as i128;
            let half = count / 2;
            let rounded = if sum >= 0 {
                (sum + half) / count
            } else {
                (sum - half) / count
            };
            RollingPoint {
                day: p.day,
                total_minor: p.total_minor,
                avg_minor: rounded as i64,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn point(total: i64) -> SeriesPoint {
        SeriesPoint {
            bucket_start: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            total_minor: total,
            purchases: 1,
            unresolved: 0,
        }
    }

    /// Month-over-month growth is computed from hand-written values, not by
    /// re-running the same arithmetic.
    #[test]
    fn growth_compares_each_bucket_with_its_predecessor() {
        let points = attach_growth(vec![point(10_000), point(12_500), point(10_000)], true);

        assert!(points[0].growth.is_none(), "nothing precedes the first bucket");

        let g1 = points[1].growth.as_ref().unwrap();
        assert_eq!(g1.previous_total, 10_000);
        assert_eq!(g1.delta_minor, 2_500);
        assert_eq!(g1.delta_pct, Some(25.0));

        let g2 = points[2].growth.as_ref().unwrap();
        assert_eq!(g2.delta_minor, -2_500);
        assert_eq!(g2.delta_pct, Some(-20.0));
    }

    /// A percentage against a zero base is undefined rather than infinite.
    #[test]
    fn growth_against_an_empty_bucket_has_no_percentage() {
        let points = attach_growth(vec![point(0), point(500)], true);
        let g = points[1].growth.as_ref().unwrap();
        assert_eq!(g.previous_total, 0);
        assert_eq!(g.delta_minor, 500);
        assert_eq!(g.delta_pct, None);
    }

    /// The trailing average rounds half away from zero, like every other money
    /// calculation in the project.
    #[test]
    fn rolling_average_rounds_half_away_from_zero() {
        let days: Vec<DailyPoint> = vec![
            DailyPoint { day: chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(), total_minor: 100 },
            DailyPoint { day: chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(), total_minor: 101 },
        ];
        let points = rolling_average(&days, 2);
        assert_eq!(points[0].avg_minor, 100, "a single day averages itself");
        assert_eq!(points[1].avg_minor, 101, "100.5 rounds up");

        let negatives = rolling_average(
            &[DailyPoint {
                day: chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                total_minor: -101,
            }],
            1,
        );
        assert_eq!(negatives[0].avg_minor, -101);
    }

    /// Non-month buckets report totals without growth.
    #[test]
    fn growth_is_only_attached_for_month_buckets() {
        let points = attach_growth(vec![point(100), point(200)], false);
        assert!(points.iter().all(|p| p.growth.is_none()));
    }
}
