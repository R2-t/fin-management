use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{IncomeRepository, WalletRepository};
use crate::model::income::{IncomeAllocation, IncomeCadence, IncomeReceipt, IncomeSchedule};

/// The caller's intent for a new income schedule.
///
/// Bundled as one named type rather than seven positional arguments: this
/// codebase has already had one real argument-order defect from same-typed
/// positionals.
#[derive(Debug, Clone)]
pub struct CreateIncomeSchedule {
    pub name: String,
    pub amount_minor: i64,
    pub currency: Option<String>,
    pub cadence: IncomeCadence,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    /// Where the money goes. Must sum to `amount_minor`.
    pub allocations: Vec<CreateAllocation>,
}

/// One wallet's share of an income schedule.
#[derive(Debug, Clone)]
pub struct CreateAllocation {
    pub wallet_id: uuid::Uuid,
    pub amount_minor: i64,
}

/// Records income expectations and their occurrences.
///
/// A schedule is an expectation, not money: it becomes real only when the user
/// records that it arrived (ADR-0008). The service owns the rules that make a
/// schedule coherent — the allocations must sum to the amount, and every wallet
/// must hold the schedule's currency.
pub struct IncomeService<I, W>
where
    I: IncomeRepository,
    W: WalletRepository,
{
    income: I,
    wallets: W,
}

impl<I, W> IncomeService<I, W>
where
    I: IncomeRepository,
    W: WalletRepository,
{
    pub fn new(income: I, wallets: W) -> Self {
        Self { income, wallets }
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<IncomeSchedule>, ApplicationError> {
        self.income.list(user_id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, schedule_id = %id))]
    pub async fn get(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<IncomeSchedule, ApplicationError> {
        self.income.get(user_id, id).await
    }

    /// Declares an income expectation.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn create(
        &self,
        user_id: uuid::Uuid,
        input: CreateIncomeSchedule,
    ) -> Result<IncomeSchedule, ApplicationError> {
        if input.name.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "income schedule name cannot be empty".into(),
            ));
        }
        if input.amount_minor <= 0 {
            return Err(ApplicationError::Validation(
                "an income schedule amount must be greater than zero".into(),
            ));
        }
        if input.allocations.is_empty() {
            return Err(ApplicationError::Validation(
                "an income schedule must direct its money somewhere".into(),
            ));
        }
        if let Some(ends_on) = input.ends_on {
            if ends_on < input.starts_on {
                return Err(ApplicationError::Validation(
                    "an income schedule cannot end before it starts".into(),
                ));
            }
        }

        let currency = match input.currency {
            Some(c) => crate::application::currency::normalize_currency(&c)?,
            None => crate::application::currency::DEFAULT_CURRENCY.to_string(),
        };

        // Every wallet must belong to the caller and hold the same currency as
        // the schedule: an income split across currencies would need a rate
        // the user has not given us (ADR-0002).
        let mut allocations = Vec::with_capacity(input.allocations.len());
        let mut total: i64 = 0;
        for allocation in &input.allocations {
            if allocation.amount_minor <= 0 {
                return Err(ApplicationError::Validation(
                    "an income allocation must be greater than zero".into(),
                ));
            }
            let wallet = self.wallets.get(user_id, allocation.wallet_id).await?;
            if wallet.currency != currency {
                return Err(ApplicationError::Validation(format!(
                    "wallet {} holds {}, but the income schedule is in {currency}",
                    wallet.name, wallet.currency
                )));
            }
            total += allocation.amount_minor;
            allocations.push(IncomeAllocation {
                id: uuid::Uuid::new_v4(),
                income_schedule_id: uuid::Uuid::nil(),
                wallet_id: allocation.wallet_id,
                amount_minor: allocation.amount_minor,
                currency: currency.clone(),
            });
        }

        // The allocations must account for the whole amount. A schedule that
        // pays out less than it says would create money on receipt; one that
        // pays out more would destroy it.
        if total != input.amount_minor {
            return Err(ApplicationError::Validation(format!(
                "the allocations total {total} but the schedule is {}",
                input.amount_minor
            )));
        }

        let id = uuid::Uuid::new_v4();
        let allocations = allocations
            .into_iter()
            .map(|mut a| {
                a.income_schedule_id = id;
                a
            })
            .collect();

        self.income
            .create(IncomeSchedule {
                id,
                user_id,
                name: input.name,
                amount_minor: input.amount_minor,
                currency,
                cadence: input.cadence,
                starts_on: input.starts_on,
                ends_on: input.ends_on,
                active: true,
                allocations,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                deleted_at: None,
            })
            .await
    }

    /// Records that the income arrived. The expectation becomes money
    /// (ADR-0008), and the ledger is credited in the same transaction.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, schedule_id = %schedule_id))]
    pub async fn receive(
        &self,
        user_id: uuid::Uuid,
        schedule_id: uuid::Uuid,
        occurred_at: Option<chrono::DateTime<chrono::Utc>>,
        timezone: &str,
    ) -> Result<IncomeReceipt, ApplicationError> {
        let occurred_at = occurred_at.unwrap_or_else(chrono::Utc::now);

        // The occurrence day is the user's day, not the server's (D25): a
        // salary that arrives at 00:30 Madrid belongs to that date.
        let occurred_on = occurred_at
            .with_timezone(&crate::application::user_profile::service::parse_timezone(timezone)?)
            .date_naive();

        self.income
            .receive(user_id, schedule_id, occurred_at, occurred_on)
            .await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, schedule_id = %id))]
    pub async fn delete(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.income.delete(user_id, id).await
    }
}
