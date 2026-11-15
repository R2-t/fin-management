use tracing::instrument;
use uuid::Uuid;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::Result;
use crate::application::inbound::repositories::AccountGroupRepository;
use crate::model::account_group::AccountGroup;
use crate::repository::{PgPool, RepositoryError};

pub struct PgAccountGroupRepo {
    pool: PgPool,
}

impl PgAccountGroupRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_group(row: &tokio_postgres::Row) -> AccountGroup {
    AccountGroup {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        deleted_at: row.get("deleted_at"),
    }
}

impl AccountGroupRepository for PgAccountGroupRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "account_group.create"))]
    async fn create(&self, user_id: Uuid, name: String) -> Result<AccountGroup> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_one(
                "INSERT INTO account_group (id, user_id, name) VALUES ($1, $2, $3) RETURNING *",
                &[&Uuid::new_v4(), &user_id, &name],
            )
            .await
            .map_err(|e| match e.code() {
                // The unique index is the rule; the message names it so the
                // client can say "you already have a group called that".
                Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) => {
                    ApplicationError::Conflict("a group with that name already exists".into())
                }
                _ => RepositoryError::DbError(e).into(),
            })?;
        Ok(row_to_group(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "account_group.list"))]
    async fn list(&self, user_id: Uuid) -> Result<Vec<AccountGroup>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let rows = conn
            .query(
                "SELECT * FROM account_group
                  WHERE user_id = $1 AND deleted_at IS NULL
                  ORDER BY lower(btrim(name))",
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(rows.iter().map(row_to_group).collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "account_group.rename"))]
    async fn rename(&self, user_id: Uuid, id: Uuid, name: String) -> Result<AccountGroup> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_opt(
                "UPDATE account_group SET name = $3, updated_at = NOW()
                  WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL
                  RETURNING *",
                &[&id, &user_id, &name],
            )
            .await
            .map_err(|e| match e.code() {
                Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) => {
                    ApplicationError::Conflict("a group with that name already exists".into())
                }
                _ => RepositoryError::DbError(e).into(),
            })?
            .ok_or_else(|| ApplicationError::NotFound("account group not found".into()))?;
        Ok(row_to_group(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "account_group.delete"))]
    async fn delete(&self, user_id: Uuid, id: Uuid) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        // Ungroup the wallets first: leaving them pointing at a soft-deleted
        // group would make them invisible to every group report while still
        // looking grouped.
        tran.execute(
            "UPDATE wallet SET account_group_id = NULL, updated_at = NOW()
              WHERE user_id = $1 AND account_group_id = $2",
            &[&user_id, &id],
        )
        .await
        .map_err(RepositoryError::DbError)?;

        let updated = tran
            .execute(
                "UPDATE account_group SET deleted_at = NOW(), updated_at = NOW()
                  WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        if updated == 0 {
            return Err(ApplicationError::NotFound("account group not found".into()));
        }

        tran.commit().await.map_err(RepositoryError::DbError)?;
        Ok(())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "account_group.assign_wallet"))]
    async fn assign_wallet(&self, user_id: Uuid, wallet_id: Uuid, group_id: Option<Uuid>) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        // One statement, so the group's ownership and the wallet's ownership are
        // checked by the same query: a group belonging to someone else simply
        // does not match the EXISTS, and nothing is written.
        let updated = conn
            .execute(
                "UPDATE wallet w SET account_group_id = $3, updated_at = NOW()
                  WHERE w.id = $1 AND w.user_id = $2 AND w.deleted_at IS NULL
                    AND ($3::uuid IS NULL OR EXISTS (
                        SELECT 1 FROM account_group g
                         WHERE g.id = $3 AND g.user_id = $2 AND g.deleted_at IS NULL))",
                &[&wallet_id, &user_id, &group_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        if updated == 0 {
            return Err(ApplicationError::NotFound("wallet or group not found".into()));
        }
        Ok(())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "account_group.wallets_in"))]
    async fn wallets_in(&self, user_id: Uuid, group_id: Uuid) -> Result<Vec<Uuid>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let rows = conn
            .query(
                "SELECT id FROM wallet
                  WHERE user_id = $1 AND account_group_id = $2 AND deleted_at IS NULL
                  ORDER BY created_at",
                &[&user_id, &group_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(rows.iter().map(|r| r.get("id")).collect())
    }
}
