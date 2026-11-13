use chrono::Datelike;
use serde::Serialize;
use tracing::instrument;

use crate::application::burn_rate::service::BurnRateService;
use crate::application::budget::period;
use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{
    AnalyticsRepository, BudgetRepository, ForecastRepository, WalletRepository,
};
use crate::model::budget::Budget;
use crate::model::forecast::{Forecast, ForecastKind};

/// The version of the forecast algorithm.
///
/// Recorded with every forecast so a result can be reproduced or superseded.
pub const MODEL_VERSION: &str = "runway-v1";

/// Whether a budget is on course to go over its limit.
#[derive(Debug, Clone, Serialize)]
pub struct OverspendForecast {
    pub budget_id: uuid::Uuid,
    pub budget_name: String,
    pub wallet_id: uuid::Uuid,
    pub currency: String,
    /// The period the forecast is about.
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    /// The limit that applies during the period.
    pub limit_minor: i64,
    /// What has been spent so far.
    pub spent_minor: i64,
    /// The days left in the period.
    pub days_remaining: i64,
    /// The pace the projection assumes. `None` when the history is too short
    /// to measure one (D42).
    pub daily_minor: Option<i64>,
    /// The spend projected for the rest of the period.
    pub projected_spend_minor: i64,
    /// The projected spend for the whole period.
    pub projected_total_minor: i64,
    /// How much over the limit the projection ends, if it does. Zero when it
    /// does not.
    pub overspend_minor: i64,
    /// Whether the projection ends over the limit.
    pub will_overspend: bool,
    /// The day the projection crosses the limit, if it does.
    pub overspend_date: Option<chrono::NaiveDate>,
    /// The assumptions the projection was made under.
    pub assumptions: Vec<String>,
}

/// Produces and persists forecasts.
///
/// Two kinds: the overspend projection for a budget, and the cash runway for a
/// wallet. Both are deterministic and computed in Rust over integer minor
/// units, so they are exact and auditable.
pub struct ForecastService<F, B, W, A>
where
    F: ForecastRepository,
    B: BudgetRepository,
    W: WalletRepository,
    A: AnalyticsRepository + Clone,
{
    forecasts: F,
    budgets: B,
    wallets: W,
    analytics: A,
    burn_rate: BurnRateService<A>,
}

impl<F, B, W, A> ForecastService<F, B, W, A>
where
    F: ForecastRepository,
    B: BudgetRepository,
    W: WalletRepository,
    A: AnalyticsRepository + Clone,
{
    pub fn new(forecasts: F, budgets: B, wallets: W, analytics: A) -> Self {
        Self {
            forecasts,
            budgets,
            wallets,
            burn_rate: BurnRateService::new(analytics.clone()),
            analytics,
        }
    }

    /// Projects whether a budget will go over its limit before the period
    /// ends.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, budget_id = %budget_id))]
    pub async fn overspend(
        &self,
        user_id: uuid::Uuid,
        budget_id: uuid::Uuid,
        on: chrono::NaiveDate,
        timezone: &str,
    ) -> Result<OverspendForecast, ApplicationError> {
        let budget = self.budgets.get(user_id, budget_id).await?;
        let wallet = self.wallets.get(user_id, budget.wallet_id).await?;

        let bounds = period::period_containing(&budget, on);
        let current = self
            .budgets
            .find_period(user_id, budget.id, bounds.start)
            .await?;
        let period = match current {
            Some(p) => p,
            None => {
                return Err(ApplicationError::NotFound(
                    "the budget's current period has not been materialised; ask for its status first".into(),
                ))
            }
        };

        // What has been spent so far in the period, and the pace.
        let spend = self
            .spend_in_period(user_id, &budget, bounds.start, bounds.end, timezone)
            .await?;
        let burn = self
            .burn_rate
            .burn_rate_over(
                user_id,
                Some(budget.wallet_id),
                &[],
                crate::application::burn_rate::service::BURN_WINDOW_DAYS,
                timezone,
            )
            .await?;
        // No measured pace means the rest of the period is not projected; the
        // assumptions say so rather than a zero pace passing for a measurement
        // (D42).
        let daily = burn.daily_minor.unwrap_or(0);

        // The days left, counting today.
        let days_remaining = (bounds.end - on).num_days() + 1;

        // The remaining days are not all the same: a weekend costs what your
        // own history says a weekend costs. When the profile is available,
        // every remaining day is priced by its weekday instead of by an
        // average (6.1). Without one, the flat pace is the honest answer.
        let projected_spend = match burn.weekday_profile.as_ref() {
            Some(profile) => {
                let mut total = 0i64;
                let mut date = on;
                while date <= bounds.end {
                    let index = date.weekday().num_days_from_monday() as usize;
                    total += profile.get(index).copied().unwrap_or(0);
                    date += chrono::Duration::days(1);
                }
                total
            }
            None => daily * days_remaining.max(0),
        };
        let projected_total = spend + projected_spend;
        let overspend = (projected_total - period.amount_minor).max(0);

        // The day the projection crosses the limit, by walking the same
        // per-day cost the projection uses rather than dividing by an
        // average.
        let overspend_date = if overspend > 0 && projected_spend > 0 {
            let remaining_before_limit = period.amount_minor - spend;
            if remaining_before_limit < 0 {
                Some(on)
            } else {
                // Walk the days, accumulating each day's cost, until the
                // cumulative spend crosses what remains of the limit.
                let per_day = || -> Vec<i64> {
                    match burn.weekday_profile.as_ref() {
                        Some(profile) => {
                            let mut costs = Vec::new();
                            let mut date = on;
                            while date <= bounds.end {
                                let index = date.weekday().num_days_from_monday() as usize;
                                costs.push(profile.get(index).copied().unwrap_or(0));
                                date += chrono::Duration::days(1);
                            }
                            costs
                        }
                        None => vec![daily; days_remaining.max(0) as usize],
                    }
                };
                let mut cumulative = 0i64;
                let mut crossing = None;
                for (offset, cost) in per_day().into_iter().enumerate() {
                    cumulative += cost;
                    if spend + cumulative > period.amount_minor {
                        crossing = Some(on + chrono::Duration::days(offset as i64));
                        break;
                    }
                }
                crossing.filter(|date| *date <= bounds.end)
            }
        } else {
            None
        };

        let mut assumptions = vec![
            format!(
                "burn rate measured over {} days of history",
                burn.history_days
            ),
            format!("burn source: {}", burn.source.as_str()),
            format!("{days_remaining} days left in the period"),
        ];
        if burn.daily_minor.is_none() {
            assumptions.push(
                "no burn rate: the history is too short to measure one, so the rest of the period is not projected"
                    .into(),
            );
        }

        let forecast = OverspendForecast {
            budget_id: budget.id,
            budget_name: budget.name.clone(),
            wallet_id: budget.wallet_id,
            currency: wallet.currency.clone(),
            period_start: bounds.start,
            period_end: bounds.end,
            limit_minor: period.amount_minor,
            spent_minor: spend,
            days_remaining,
            daily_minor: burn.daily_minor,
            projected_spend_minor: projected_spend,
            projected_total_minor: projected_total,
            overspend_minor: overspend,
            will_overspend: overspend > 0,
            overspend_date,
            assumptions: std::mem::take(&mut assumptions),
        };

        // Persist it, so a forecast can be compared and back-tested later.
        self.forecasts
            .insert(Forecast {
                id: uuid::Uuid::new_v4(),
                user_id,
                wallet_id: budget.wallet_id,
                kind: ForecastKind::BudgetOverspend,
                generated_at: chrono::Utc::now(),
                horizon_days: days_remaining,
                result: serde_json::to_value(&forecast)
                    .map_err(ApplicationError::Serialization)?,
                assumptions: serde_json::to_value(&forecast.assumptions)
                    .map_err(ApplicationError::Serialization)?,
                model_version: MODEL_VERSION.to_string(),
            })
            .await?;

        Ok(forecast)
    }

    /// The user's persisted forecasts, newest first.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(
        &self,
        user_id: uuid::Uuid,
        kind: Option<ForecastKind>,
    ) -> Result<Vec<Forecast>, ApplicationError> {
        self.forecasts.list(user_id, kind).await
    }

    /// The spend in a period, in the budget's category and wallet.
    ///
    /// The DAILY series is summed rather than a monthly bucket: a budget's
    /// period does not align with the calendar month (a reset day of 15 runs
    /// from the 15th to the 14th), so a monthly aggregate would count the
    /// wrong days.
    async fn spend_in_period(
        &self,
        user_id: uuid::Uuid,
        budget: &Budget,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
        timezone: &str,
    ) -> Result<i64, ApplicationError> {
        let range = crate::application::inbound::repositories::SpendRange {
            from: crate::application::user_profile::service::day_start(from, timezone)?,
            to: crate::application::user_profile::service::day_start(
                to + chrono::Duration::days(1),
                timezone,
            )?,
            wallet_id: Some(budget.wallet_id),
            category_id: budget.category_id,
            currency: None,
            timezone: timezone.to_string(),
        };

        let series = self.analytics.daily_spend(user_id, &range, 0).await?;

        Ok(series
            .into_iter()
            .flat_map(|s| s.points)
            .filter(|p| p.day >= from && p.day <= to)
            .map(|p| p.total_minor)
            .sum())
    }
}
