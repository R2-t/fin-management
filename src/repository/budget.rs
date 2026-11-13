use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{BudgetRepository, Result};
use crate::model::budget::{Budget, BudgetPeriod};
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgBudgetRepo {
    pool: PgPool,
}

impl PgBudgetRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_budget(row: &tokio_postgres::Row) -> Budget {
    Budget {
        id: row.get("id"),
        user_id: row.get("user_id"),
        wallet_id: row.get("wallet_id"),
        category_id: row.get("category_id"),
        name: row.get("name"),
        amount_minor: row.get("amount_minor"),
        cadence: crate::model::budget::BudgetCadence::parse(row.get("cadence"))
            .unwrap_or(crate::model::budget::BudgetCadence::Monthly),
        reset_day: row.get("reset_day"),
        starts_on: row.get("starts_on"),
        ends_on: row.get("ends_on"),
        rollover_unspent: row.get("rollover_unspent"),
        carry_overspend: row.get("carry_overspend"),
        active: row.get("active"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}

fn row_to_period(row: &tokio_postgres::Row) -> BudgetPeriod {
    BudgetPeriod {
        id: row.get("id"),
        budget_id: row.get("budget_id"),
        user_id: row.get("user_id"),
        amount_minor: row.get("amount_minor"),
        template_minor: row.get("template_minor"),
        unspent_minor: row.get("unspent_minor"),
        period_start: row.get("period_start"),
        period_end: row.get("period_end"),
        created_at: row.get("created_at"),
    }
}

impl BudgetRepository for PgBudgetRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "budget.create"))]
    async fn create(&self, budget: Budget) -> Result<Budget> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO budget
                    (id, user_id, wallet_id, category_id, name, amount_minor, cadence, reset_day,
                     starts_on, ends_on, rollover_unspent, carry_overspend, active)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                   RETURNING *"#,
                &[
                    &budget.id,
                    &budget.user_id,
                    &budget.wallet_id,
                    &budget.category_id,
                    &budget.name,
                    &budget.amount_minor,
                    &budget.cadence.as_str(),
                    &budget.reset_day,
                    &budget.starts_on,
                    &budget.ends_on,
                    &budget.rollover_unspent,
                    &budget.carry_overspend,
                    &budget.active,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_budget(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "budget.list"))]
    async fn list(&self, user_id: Uuid) -> Result<Vec<Budget>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                "SELECT * FROM budget WHERE user_id = $1 AND deleted_at IS NULL ORDER BY name",
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_budget).collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "budget.get"))]
    async fn get(&self, user_id: Uuid, id: Uuid) -> Result<Budget> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT * FROM budget WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_budget(&row))
    }

    /// The period starting on `period_start`, if it has been materialised.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "budget.find_period"))]
    async fn find_period(
        &self,
        user_id: Uuid,
        budget_id: Uuid,
        period_start: chrono::NaiveDate,
    ) -> Result<Option<BudgetPeriod>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT * FROM budget_period
                  WHERE budget_id = $1 AND user_id = $2 AND period_start = $3",
                &[&budget_id, &user_id, &period_start],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row.as_ref().map(row_to_period))
    }

    /// Materialises a period.
    ///
    /// The unique constraint on (budget_id, period_start) makes this
    /// idempotent: two concurrent requests for the same period produce one
    /// row, not two.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "budget.insert_period"))]
    async fn insert_period(&self, period: BudgetPeriod) -> Result<BudgetPeriod> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO budget_period
                    (id, budget_id, user_id, amount_minor, template_minor, unspent_minor,
                     period_start, period_end)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                   ON CONFLICT (budget_id, period_start) DO UPDATE SET amount_minor = EXCLUDED.amount_minor
                   RETURNING *"#,
                &[
                    &period.id,
                    &period.budget_id,
                    &period.user_id,
                    &period.amount_minor,
                    &period.template_minor,
                    &period.unspent_minor,
                    &period.period_start,
                    &period.period_end,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_period(&row))
    }

    /// Sets a period's limit. An edit applies to that period only (ADR-0011).
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "budget.set_period_limit"))]
    async fn set_period_limit(
        &self,
        user_id: Uuid,
        budget_id: Uuid,
        period_start: chrono::NaiveDate,
        amount_minor: i64,
    ) -> Result<BudgetPeriod> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "UPDATE budget_period SET amount_minor = $4
                  WHERE budget_id = $1 AND user_id = $2 AND period_start = $3
                  RETURNING *",
                &[&budget_id, &user_id, &period_start, &amount_minor],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_period(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "budget.delete"))]
    async fn delete(&self, user_id: Uuid, id: Uuid) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let n = conn
            .execute(
                "UPDATE budget SET deleted_at = NOW() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
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
