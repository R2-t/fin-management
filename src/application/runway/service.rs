use crate::application::runway::projection::{project, DayMovements};
use serde::Serialize;
use tracing::instrument;

use crate::application::burn_rate::service::{BurnRate, BurnRateService, MIN_BURN_HISTORY_DAYS};
use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{AnalyticsRepository, LedgerRepository};
use crate::application::recurring::matcher;
use crate::model::income::IncomeSchedule;
use crate::model::recurring::{RecurringRule, RecurringStatus};

/// How far ahead the runway projects.
pub const HORIZON_DAYS: i64 = 90;

/// The runway projection for one Wallet.
#[derive(Debug, Clone, Serialize)]
pub struct Runway {
    pub wallet_id: uuid::Uuid,
    pub currency: String,
    /// When this answer was computed.
    pub as_of: chrono::DateTime<chrono::Utc>,
    /// The balance the projection started from: the ledger's sum.
    pub starting_balance_minor: i64,
    /// The lowest balance the projection reaches.
    pub trough_minor: i64,
    /// The day the trough falls on.
    pub trough_date: chrono::NaiveDate,
    /// The day the balance returns above zero. `None` when it never does
    /// within the horizon.
    pub recovery_date: Option<chrono::NaiveDate>,
    /// The balance at the end of the horizon.
    pub projected_balance_minor: i64,
    /// The horizon the projection covered.
    pub horizon_days: i64,
    /// The pace the projection assumed.
    pub burn_rate: BurnRate,
    /// The assumptions the projection was made under, so a stale forecast
    /// explains itself.
    pub assumptions: Vec<String>,
}

/// Projects how long a Wallet's money lasts.
///
/// The projection walks forward day by day from today's ledger balance:
/// each day subtracts the projected discretionary spending and the recurring
/// charges due that day, and adds the scheduled income due that day. It
/// reports the trough, the lowest balance reached, and the recovery date, the
/// day the balance returns above zero (D38).
///
/// The projection runs forward even when the balance is already negative: a
/// balance below zero sharpens the question rather than removing it. The user
/// is already overdrawn, and what they need to know is how deep the hole gets
/// and whether the next paycheck covers it.
pub struct RunwayService<A, L>
where
    A: AnalyticsRepository,
    L: LedgerRepository,
{
    ledger: L,
    burn_rate: BurnRateService<A>,
}

impl<A, L> RunwayService<A, L>
where
    A: AnalyticsRepository + Clone,
    L: LedgerRepository,
{
    pub fn new(analytics: A, ledger: L) -> Self {
        Self {
            burn_rate: BurnRateService::new(analytics),
            ledger,
        }
    }

    /// Projects the runway for one Wallet.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, wallet_id = %wallet_id))]
    pub async fn runway(
        &self,
        user_id: uuid::Uuid,
        wallet_id: uuid::Uuid,
        rules: &[RecurringRule],
        incomes: &[IncomeSchedule],
    ) -> Result<Runway, ApplicationError> {
        let today = chrono::Utc::now().date_naive();
        let horizon_end = today + chrono::Duration::days(HORIZON_DAYS - 1);

        // The starting balance: the ledger's sum (ADR-0004).
        let balance = self.ledger.balance(user_id, wallet_id).await?;

        // The pace. Measured over the trailing window, which is also the
        // span the projection covers.
        let burn = self
            .burn_rate
            .burn_rate_over(user_id, Some(wallet_id), rules, crate::application::burn_rate::service::BURN_WINDOW_DAYS)
            .await?;

        // The recurring charges due in the horizon, per day.
        let mut recurring_by_day: Vec<(chrono::NaiveDate, i64)> = Vec::new();
        for rule in rules {
            // A cancelled rule is no longer forecast: it stops being a
            // commitment the day the user cancels it.
            if rule.status != RecurringStatus::Active {
                continue;
            }
            // Only a rule on THIS wallet affects this wallet.
            if rule.wallet_id != wallet_id {
                continue;
            }
            if rule.currency != balance.currency {
                // A rule in another currency would need a rate we do not have
                // (ADR-0002). It is not projected.
                continue;
            }
            for date in matcher::occurrences_in_range(rule, today, horizon_end) {
                add_to_day(&mut recurring_by_day, date, rule.amount_minor);
            }
        }

        // The scheduled income due in the horizon, per day.
        let mut income_by_day: Vec<(chrono::NaiveDate, i64)> = Vec::new();
        for income in incomes {
            if !income.active {
                continue;
            }
            for allocation in &income.allocations {
                if allocation.wallet_id != wallet_id {
                    continue;
                }
                if allocation.currency != balance.currency {
                    continue;
                }
                let dates = income_dates(income, today, horizon_end);
                for date in dates {
                    add_to_day(&mut income_by_day, date, allocation.amount_minor);
                }
            }
        }

        // The projection itself, through the pure core so the property tests
        // exercise the same code the service runs.
        let movements: Vec<DayMovements> = (1..HORIZON_DAYS)
            .map(|offset| {
                let date = today + chrono::Duration::days(offset);
                DayMovements {
                    recurring_minor: take_from_day(&mut recurring_by_day, date),
                    income_minor: take_from_day(&mut income_by_day, date),
                }
            })
            .collect();

        let projection = project(
            balance.balance_minor,
            today,
            &movements,
            burn.weekday_profile.as_deref(),
            // No measured pace means no discretionary spending is projected.
            // The assumption below says so, rather than a zero pace passing
            // for a measurement (D42).
            burn.daily_minor.unwrap_or(0),
        );

        let current = projection.ending_balance_minor;
        let trough = projection.trough_minor;
        let trough_date = projection.trough_date;
        let recovery_date = projection.recovery_date;

        let mut assumptions = vec![
            format!(
                "burn rate measured over {} days of history",
                burn.history_days
            ),
            format!("burn source: {}", burn.source.as_str()),
            format!("horizon: {HORIZON_DAYS} days"),
        ];
        if burn.weekday_profile.is_some() {
            assumptions.push("daily spending follows the weekday median profile".into());
        }
        if burn.daily_minor.is_none() {
            assumptions.push(format!(
                "no burn rate: only {MIN_BURN_HISTORY_DAYS} days of history are needed and {} are available, so discretionary spending is not projected",
                burn.history_days
            ));
        }
        if rules.is_empty() {
            assumptions.push("no recurring charges declared".into());
        }
        if incomes.is_empty() {
            assumptions.push("no income scheduled".into());
        }

        Ok(Runway {
            wallet_id,
            currency: balance.currency,
            as_of: chrono::Utc::now(),
            starting_balance_minor: balance.balance_minor,
            trough_minor: trough,
            trough_date,
            recovery_date,
            projected_balance_minor: current,
            horizon_days: HORIZON_DAYS,
            burn_rate: burn,
            assumptions,
        })
    }
}

/// The dates an income schedule expects an occurrence on.
fn income_dates(
    income: &IncomeSchedule,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> Vec<chrono::NaiveDate> {
    use crate::model::recurring::RecurrenceCadence;
    let cadence = match income.cadence {
        crate::model::income::IncomeCadence::Weekly => RecurrenceCadence::Weekly,
        crate::model::income::IncomeCadence::Biweekly => RecurrenceCadence::Biweekly,
        crate::model::income::IncomeCadence::Semimonthly => RecurrenceCadence::Semimonthly,
        crate::model::income::IncomeCadence::Monthly => RecurrenceCadence::Monthly,
    };
    let mut dates = cadence.occurrences(income.starts_on, from, to);
    // A schedule that has ended stops producing occurrences.
    if let Some(ends_on) = income.ends_on {
        dates.retain(|d| *d <= ends_on);
    }
    dates
}

/// Adds to the amount due on a day.
fn add_to_day(days: &mut Vec<(chrono::NaiveDate, i64)>, date: chrono::NaiveDate, amount: i64) {
    match days.iter_mut().find(|(d, _)| *d == date) {
        Some((_, total)) => *total += amount,
        None => days.push((date, amount)),
    }
}

/// The amount due on a day, and removes it so it is not counted twice.
fn take_from_day(days: &mut [(chrono::NaiveDate, i64)], date: chrono::NaiveDate) -> i64 {
    match days.iter_mut().find(|(d, _)| *d == date) {
        Some((_, total)) => {
            let amount = *total;
            *total = 0;
            amount
        }
        None => 0,
    }
}
