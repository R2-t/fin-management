use tracing::instrument;

use crate::application::budget::period;
use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{
    AnalyticsRepository, BudgetRepository, WalletRepository,
};
use crate::model::budget::{Budget, BudgetCadence, BudgetPeriod, BudgetStatus};

/// The caller's intent for a new budget.
#[derive(Debug, Clone)]
pub struct CreateBudget {
    pub wallet_id: uuid::Uuid,
    /// `None` means the limit covers the whole Wallet.
    pub category_id: Option<i32>,
    pub name: String,
    pub amount_minor: i64,
    pub cadence: BudgetCadence,
    /// The day the period starts: 1 for a calendar month, 15 for a mid-month
    /// payday, -1 for a period ending on the last day of the month.
    pub reset_day: i16,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    pub rollover_unspent: bool,
    pub carry_overspend: bool,
}

/// Manages budgets and their periods.
///
/// A Budget is a template plus materialised periods (ADR-0009). Periods are
/// created lazily, when a period is first asked about, and the service owns
/// the rules that decide a period's effective limit: a carried overspend
/// lowers it, and nothing raises it (ADR-0010).
pub struct BudgetService<B, W, A>
where
    B: BudgetRepository,
    W: WalletRepository,
    A: AnalyticsRepository,
{
    budgets: B,
    wallets: W,
    analytics: A,
}

impl<B, W, A> BudgetService<B, W, A>
where
    B: BudgetRepository,
    W: WalletRepository,
    A: AnalyticsRepository,
{
    pub fn new(budgets: B, wallets: W, analytics: A) -> Self {
        Self {
            budgets,
            wallets,
            analytics,
        }
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<Budget>, ApplicationError> {
        self.budgets.list(user_id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, budget_id = %id))]
    pub async fn get(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<Budget, ApplicationError> {
        self.budgets.get(user_id, id).await
    }

    /// Declares a budget.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn create(
        &self,
        user_id: uuid::Uuid,
        input: CreateBudget,
    ) -> Result<Budget, ApplicationError> {
        if input.name.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "a budget name cannot be empty".into(),
            ));
        }
        if input.amount_minor <= 0 {
            return Err(ApplicationError::Validation(
                "a budget amount must be greater than zero".into(),
            ));
        }
        // A reset day of 29-31 is rejected rather than clamped: the period
        // arithmetic clamps to the 28th so that every month has that day, and
        // silently giving the user the 28th when they asked for the 31st would
        // put spending in the wrong period. -1 is how a month-end payday is
        // expressed (ADR-0009).
        if !(-1..=28).contains(&input.reset_day) {
            return Err(ApplicationError::Validation(
                "a reset day must be between 1 and 28, or -1 for the last day of the month".into(),
            ));
        }
        if let Some(ends_on) = input.ends_on {
            if ends_on < input.starts_on {
                return Err(ApplicationError::Validation(
                    "a budget cannot end before it starts".into(),
                ));
            }
        }

        // The wallet must belong to the caller. Its currency is authoritative:
        // the limit is compared against spend in that currency (ADR-0002).
        self.wallets.get(user_id, input.wallet_id).await?;

        self.budgets
            .create(Budget {
                id: uuid::Uuid::new_v4(),
                user_id,
                wallet_id: input.wallet_id,
                category_id: input.category_id,
                name: input.name,
                amount_minor: input.amount_minor,
                cadence: input.cadence,
                reset_day: input.reset_day,
                starts_on: input.starts_on,
                ends_on: input.ends_on,
                rollover_unspent: input.rollover_unspent,
                carry_overspend: input.carry_overspend,
                active: true,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                deleted_at: None,
            })
            .await
    }

    /// The budget's current period and how it is going.
    ///
    /// The period is materialised on first ask: it is created with the
    /// effective limit the carry rules give it, which needs the previous
    /// period's final spend.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, budget_id = %id))]
    pub async fn current_status(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
        on: chrono::NaiveDate,
        timezone: &str,
    ) -> Result<BudgetStatus, ApplicationError> {
        let budget = self.budgets.get(user_id, id).await?;
        let wallet = self.wallets.get(user_id, budget.wallet_id).await?;

        let bounds = period::period_containing(&budget, on);
        let period = self
            .materialise(user_id, &budget, bounds.start, bounds.end, timezone)
            .await?;

        let spent = self
            .spend_in_period(user_id, &budget, period.period_start, period.period_end, timezone)
            .await?;

        // The unspent money is reported, never carried into the next limit
        // (ADR-0010).
        let unspent = period::unspent(period.amount_minor, spent);
        let remaining = period.amount_minor - spent;

        Ok(BudgetStatus {
            period: BudgetPeriod {
                unspent_minor: unspent,
                ..period
            },
            currency: wallet.currency,
            spent_minor: spent,
            remaining_minor: remaining,
            over_budget: remaining < 0,
        })
    }

    /// Sets a period's limit. The edit applies to that period only
    /// (ADR-0011).
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, budget_id = %id))]
    pub async fn set_period_limit(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
        period_start: chrono::NaiveDate,
        amount_minor: i64,
        timezone: &str,
    ) -> Result<BudgetPeriod, ApplicationError> {
        let budget = self.budgets.get(user_id, id).await?;

        // The period must exist before it can be edited.
        let bounds = period::period_containing(&budget, period_start);
        self.materialise(user_id, &budget, bounds.start, bounds.end, timezone)
            .await?;

        self.budgets
            .set_period_limit(user_id, id, bounds.start, amount_minor)
            .await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, budget_id = %id))]
    pub async fn delete(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.budgets.delete(user_id, id).await
    }

    /// Returns the period, creating it if it does not exist.
    async fn materialise(
        &self,
        user_id: uuid::Uuid,
        budget: &Budget,
        period_start: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
        timezone: &str,
    ) -> Result<BudgetPeriod, ApplicationError> {
        if let Some(existing) = self
            .budgets
            .find_period(user_id, budget.id, period_start)
            .await?
        {
            return Ok(existing);
        }

        // The effective limit depends on the previous period's final spend.
        let previous = period::previous_period(budget, period_start);
        let previous_spend = self
            .spend_in_period(user_id, budget, previous.start, previous.end, timezone)
            .await?;

        let amount_minor = period::effective_limit(
            budget.amount_minor,
            Some(previous_spend),
            budget.carry_overspend,
        );

        self.budgets
            .insert_period(BudgetPeriod {
                id: uuid::Uuid::new_v4(),
                budget_id: budget.id,
                user_id,
                amount_minor,
                template_minor: budget.amount_minor,
                unspent_minor: 0,
                period_start,
                period_end,
                created_at: chrono::Utc::now(),
            })
            .await
    }

    /// The spend in a period: in the budget's category, in its wallet, in the
    /// wallet's currency.
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
