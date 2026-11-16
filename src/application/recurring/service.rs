use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{RecurringRuleRepository, WalletRepository};
use crate::model::recurring::{RecurringRule, RecurringStatus, RecurrenceCadence};

/// The caller's intent for a new recurring rule.
#[derive(Debug, Clone)]
pub struct CreateRecurringRule {
    pub wallet_id: uuid::Uuid,
    /// The merchant it is paid to. Optional: the user may not know or care.
    pub merchant_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    pub name: String,
    pub amount_minor: i64,
    pub currency: Option<String>,
    pub cadence: RecurrenceCadence,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
}

/// Declares recurring commitments.
///
/// A rule is a *declaration*, not a fact about money: nothing is written to
/// the ledger until the user records the actual purchase. The service owns the
/// rules that make a rule coherent -- the wallet must belong to the caller and
/// hold the rule's currency, and the amount must be positive.
pub struct RecurringRuleService<R, W>
where
    R: RecurringRuleRepository,
    W: WalletRepository,
{
    rules: R,
    wallets: W,
}

impl<R, W> RecurringRuleService<R, W>
where
    R: RecurringRuleRepository,
    W: WalletRepository,
{
    pub fn new(rules: R, wallets: W) -> Self {
        Self { rules, wallets }
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn list(&self, user_id: uuid::Uuid) -> Result<Vec<RecurringRule>, ApplicationError> {
        self.rules.list(user_id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, rule_id = %id))]
    pub async fn get(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<RecurringRule, ApplicationError> {
        self.rules.get(user_id, id).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id))]
    pub async fn create(
        &self,
        user_id: uuid::Uuid,
        input: CreateRecurringRule,
    ) -> Result<RecurringRule, ApplicationError> {
        if input.name.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "a recurring rule name cannot be empty".into(),
            ));
        }
        if input.amount_minor <= 0 {
            return Err(ApplicationError::Validation(
                "a recurring rule amount must be greater than zero".into(),
            ));
        }
        if let Some(ends_on) = input.ends_on {
            if ends_on < input.starts_on {
                return Err(ApplicationError::Validation(
                    "a recurring rule cannot end before it starts".into(),
                ));
            }
        }

        // The wallet must belong to the caller. Its currency is authoritative:
        // a rule in another currency would need a rate the user has not given
        // us (ADR-0002).
        let wallet = self.wallets.get(user_id, input.wallet_id).await?;
        let currency = match input.currency {
            Some(c) => crate::application::currency::normalize_currency(&c)?,
            None => wallet.currency.clone(),
        };
        if currency != wallet.currency {
            return Err(ApplicationError::Validation(format!(
                "wallet {} holds {}, so a recurring rule cannot be in {currency}",
                wallet.name, wallet.currency
            )));
        }

        self.rules
            .create(RecurringRule {
                id: uuid::Uuid::new_v4(),
                user_id,
                wallet_id: input.wallet_id,
                merchant_id: input.merchant_id,
                category_id: input.category_id,
                name: input.name,
                amount_minor: input.amount_minor,
                currency,
                cadence: input.cadence,
                starts_on: input.starts_on,
                ends_on: input.ends_on,
                status: RecurringStatus::Active,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                deleted_at: None,
            })
            .await
    }

    /// Cancels a rule. It stops being forecast but keeps its history.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, rule_id = %id))]
    pub async fn cancel(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<RecurringRule, ApplicationError> {
        self.rules.set_status(user_id, id, RecurringStatus::Cancelled).await
    }

    /// Reactivates a cancelled rule.
    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, rule_id = %id))]
    pub async fn reactivate(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<RecurringRule, ApplicationError> {
        self.rules.set_status(user_id, id, RecurringStatus::Active).await
    }

    #[instrument(skip_all, err(level = "warn"), fields(user_id = %user_id, rule_id = %id))]
    pub async fn delete(
        &self,
        user_id: uuid::Uuid,
        id: uuid::Uuid,
    ) -> Result<(), ApplicationError> {
        self.rules.delete(user_id, id).await
    }
}
