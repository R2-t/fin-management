use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{IncomeRepository, Result};
use crate::model::income::{IncomeAllocation, IncomeCadence, IncomeReceipt, IncomeSchedule};
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgIncomeRepo {
    pool: PgPool,
}

impl PgIncomeRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_schedule(row: &tokio_postgres::Row) -> IncomeSchedule {
    IncomeSchedule {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
        cadence: IncomeCadence::parse(row.get("cadence")).unwrap_or(IncomeCadence::Monthly),
        starts_on: row.get("starts_on"),
        ends_on: row.get("ends_on"),
        active: row.get("active"),
        allocations: Vec::new(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}

fn row_to_allocation(row: &tokio_postgres::Row) -> IncomeAllocation {
    IncomeAllocation {
        id: row.get("id"),
        income_schedule_id: row.get("income_schedule_id"),
        wallet_id: row.get("wallet_id"),
        amount_minor: row.get("amount_minor"),
        currency: row.get("currency"),
    }
}

impl IncomeRepository for PgIncomeRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "income.create"))]
    async fn create(&self, schedule: IncomeSchedule) -> Result<IncomeSchedule> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let row = tran
            .query_one(
                r#"INSERT INTO income_schedule
                    (id, user_id, name, amount_minor, currency, cadence, starts_on, ends_on, active)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                   RETURNING *"#,
                &[
                    &schedule.id,
                    &schedule.user_id,
                    &schedule.name,
                    &schedule.amount_minor,
                    &schedule.currency,
                    &schedule.cadence.as_str(),
                    &schedule.starts_on,
                    &schedule.ends_on,
                    &schedule.active,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        for allocation in &schedule.allocations {
            tran.execute(
                r#"INSERT INTO income_allocation
                       (income_schedule_id, user_id, wallet_id, amount_minor, currency)
                   VALUES ($1, $2, $3, $4, $5)"#,
                &[
                    &schedule.id,
                    &schedule.user_id,
                    &allocation.wallet_id,
                    &allocation.amount_minor,
                    &allocation.currency,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        }

        tran.commit().await.map_err(RepositoryError::DbError)?;

        let mut created = row_to_schedule(&row);
        created.allocations = schedule.allocations;
        Ok(created)
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "income.list"))]
    async fn list(&self, user_id: Uuid) -> Result<Vec<IncomeSchedule>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                "SELECT * FROM income_schedule WHERE user_id = $1 AND deleted_at IS NULL ORDER BY name",
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        let mut schedules: Vec<IncomeSchedule> = rows.iter().map(row_to_schedule).collect();

        // The allocations are loaded in one query rather than per schedule.
        let allocation_rows = conn
            .query(
                "SELECT * FROM income_allocation WHERE user_id = $1 ORDER BY income_schedule_id",
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        for allocation in allocation_rows.iter().map(row_to_allocation) {
            if let Some(schedule) = schedules
                .iter_mut()
                .find(|s| s.id == allocation.income_schedule_id)
            {
                schedule.allocations.push(allocation);
            }
        }

        Ok(schedules)
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "income.get"))]
    async fn get(&self, user_id: Uuid, id: Uuid) -> Result<IncomeSchedule> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT * FROM income_schedule WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        let mut schedule = row_to_schedule(&row);

        let allocation_rows = conn
            .query(
                "SELECT * FROM income_allocation WHERE income_schedule_id = $1 ORDER BY id",
                &[&id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        schedule.allocations = allocation_rows.iter().map(row_to_allocation).collect();

        Ok(schedule)
    }

    /// Records an occurrence: the expectation becomes money (ADR-0008).
    ///
    /// The ledger credits are written in the same transaction as the receipt,
    /// so a recorded occurrence and the money it represents cannot disagree.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "income.receive"))]
    async fn receive(
        &self,
        user_id: Uuid,
        schedule_id: Uuid,
        occurred_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<IncomeReceipt> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        let schedule = tran
            .query_opt(
                "SELECT * FROM income_schedule WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL AND active",
                &[&schedule_id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        let schedule = row_to_schedule(&schedule);

        let allocation_rows = tran
            .query(
                "SELECT * FROM income_allocation WHERE income_schedule_id = $1",
                &[&schedule_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        let receipt = tran
            .query_one(
                r#"INSERT INTO income_receipt
                    (income_schedule_id, user_id, occurred_at, amount_minor, currency)
                   VALUES ($1, $2, $3, $4, $5)
                   RETURNING *"#,
                &[
                    &schedule_id,
                    &user_id,
                    &occurred_at,
                    &schedule.amount_minor,
                    &schedule.currency,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        // The credits. One per allocation, so a split across wallets writes one
        // entry per wallet.
        for allocation_row in &allocation_rows {
            let allocation = row_to_allocation(allocation_row);
            tran.execute(
                r#"INSERT INTO ledger_entry
                       (user_id, wallet_id, occurred_at, amount_minor, currency, kind)
                   VALUES ($1, $2, $3, $4, $5, 'income')"#,
                &[
                    &user_id,
                    &allocation.wallet_id,
                    &occurred_at,
                    &allocation.amount_minor,
                    &allocation.currency,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        }

        tran.commit().await.map_err(RepositoryError::DbError)?;

        Ok(IncomeReceipt {
            id: receipt.get("id"),
            income_schedule_id: receipt.get("income_schedule_id"),
            user_id: receipt.get("user_id"),
            occurred_at: receipt.get("occurred_at"),
            amount_minor: receipt.get("amount_minor"),
            currency: receipt.get("currency"),
            created_at: receipt.get("created_at"),
        })
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "income.delete"))]
    async fn delete(&self, user_id: Uuid, id: Uuid) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let n = conn
            .execute(
                "UPDATE income_schedule SET deleted_at = NOW() WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
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
