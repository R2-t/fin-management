use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{RecurringRuleRepository, Result};
use crate::model::recurring::{RecurringRule, RecurringStatus, RecurrenceCadence};
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgRecurringRuleRepo {
    pool: PgPool,
}

impl PgRecurringRuleRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_rule(row: &tokio_postgres::Row) -> RecurringRule {
    RecurringRule {
        id: row.get("id"),
        user_id: row.get("user_id"),
        wallet_id: row.get("wallet_id"),
        merchant_id: row.get("merchant_id"),
        category_id: row.get("category_id"),
        name: row.get("name"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        cadence: RecurrenceCadence::parse(row.get("cadence")).unwrap_or(RecurrenceCadence::Monthly),
        starts_on: row.get("starts_on"),
        ends_on: row.get("ends_on"),
        status: RecurringStatus::parse(row.get("status")).unwrap_or(RecurringStatus::Active),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}

impl RecurringRuleRepository for PgRecurringRuleRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "recurring.create"))]
    async fn create(&self, rule: RecurringRule) -> Result<RecurringRule> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO recurring_rule
                    (id, user_id, wallet_id, merchant_id, category_id, name, amount_minor,
                     currency, cadence, starts_on, ends_on, status)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                   RETURNING *"#,
                &[
                    &rule.id,
                    &rule.user_id,
                    &rule.wallet_id,
                    &rule.merchant_id,
                    &rule.category_id,
                    &rule.name,
                    &rule.amount_minor,
                    &rule.currency,
                    &rule.cadence.as_str(),
                    &rule.starts_on,
                    &rule.ends_on,
                    &rule.status.as_str(),
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_rule(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "recurring.list"))]
    async fn list(&self, user_id: Uuid) -> Result<Vec<RecurringRule>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                "SELECT * FROM recurring_rule
                  WHERE user_id = $1 AND deleted_at IS NULL
                  ORDER BY name",
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_rule).collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "recurring.get"))]
    async fn get(&self, user_id: Uuid, id: Uuid) -> Result<RecurringRule> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT * FROM recurring_rule WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_rule(&row))
    }

    /// Sets a rule's status. A cancelled rule stops being forecast but keeps
    /// its history: the user may cancel a subscription and later restart it.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "recurring.set_status"))]
    async fn set_status(
        &self,
        user_id: Uuid,
        id: Uuid,
        status: RecurringStatus,
    ) -> Result<RecurringRule> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "UPDATE recurring_rule SET status = $3, updated_at = NOW()
                  WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
                  RETURNING *",
                &[&id, &user_id, &status.as_str()],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_rule(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "recurring.delete"))]
    async fn delete(&self, user_id: Uuid, id: Uuid) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let n = conn
            .execute(
                "UPDATE recurring_rule SET deleted_at = NOW() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        if n == 0 {
            return Err(RepositoryError::NotFound.into());
        }
        Ok(())
    }
}
