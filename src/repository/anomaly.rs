use tracing::instrument;
use uuid::Uuid;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{
    AnomalyRepository, CreateAnomalyParams, Result,
};
use crate::model::anomaly::{Anomaly, AnomalyStatus};
use crate::repository::{PgPool, RepositoryError};

pub struct PgAnomalyRepo {
    pool: PgPool,
}

impl PgAnomalyRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_anomaly(row: &tokio_postgres::Row) -> Anomaly {
    Anomaly {
        id: row.get("id"),
        user_id: row.get("user_id"),
        kind: row.get("kind"),
        subject_type: row.get("subject_type"),
        subject_id: row.get("subject_id"),
        title: row.get("title"),
        detail: row.get("detail"),
        severity: row.get("severity"),
        detected_at: row.get("detected_at"),
        key: row.get("key"),
        dismissed_at: row.get("dismissed_at"),
        dismissed_reason: row.get("dismissed_reason"),
    }
}

impl AnomalyRepository for PgAnomalyRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "anomaly.list"))]
    async fn list(
        &self,
        user_id: Uuid,
        kind: Option<&str>,
        state: Option<AnomalyStatus>,
        limit: i64,
        before: Option<(chrono::DateTime<chrono::Utc>, Uuid)>,
    ) -> Result<Vec<Anomaly>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let state = state.map(AnomalyStatus::as_str);
        let (before_at, before_id) = match before {
            Some((at, id)) => (Some(at), Some(id)),
            None => (None, None),
        };
        let rows = conn
            .query(
                "SELECT * FROM anomaly
                  WHERE user_id = $1
                    AND ($2::text IS NULL OR kind = $2)
                    AND ($3::text IS NULL
                         OR ($3 = 'open' AND dismissed_at IS NULL)
                         OR ($3 = 'dismissed' AND dismissed_at IS NOT NULL))
                    AND ($4::timestamptz IS NULL
                         OR detected_at < $4
                         OR (detected_at = $4 AND id < $5))
                  ORDER BY detected_at DESC, id DESC
                  LIMIT $6",
                &[&user_id, &kind, &state, &before_at, &before_id, &limit],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(rows.iter().map(row_to_anomaly).collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "anomaly.create"))]
    async fn create(&self, user_id: Uuid, params: CreateAnomalyParams) -> Result<Anomaly> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let inserted = conn
            .query_opt(
                "INSERT INTO anomaly
                    (id, user_id, kind, subject_type, subject_id, title, detail,
                     severity, detected_at, key)
                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, COALESCE($9, NOW()), $10)
                  ON CONFLICT DO NOTHING
                  RETURNING *",
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &params.kind,
                    &params.subject_type,
                    &params.subject_id,
                    &params.title,
                    &params.detail,
                    &params.severity.as_str(),
                    &params.detected_at,
                    &params.key,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        if let Some(row) = inserted {
            return Ok(row_to_anomaly(&row));
        }

        // The unique index is the idempotency rule: this evidence already has a
        // finding, so return it rather than raising a second one.
        let row = conn
            .query_opt(
                "SELECT * FROM anomaly
                  WHERE user_id = $1 AND kind = $2 AND subject_type = $3
                    AND subject_id IS NOT DISTINCT FROM $4 AND key = $5",
                &[
                    &user_id,
                    &params.kind,
                    &params.subject_type,
                    &params.subject_id,
                    &params.key,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or_else(|| {
                ApplicationError::Internal("anomaly insert conflicted but no row matched".into())
            })?;
        Ok(row_to_anomaly(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "anomaly.dismiss"))]
    async fn dismiss(&self, user_id: Uuid, id: Uuid, reason: Option<String>) -> Result<Anomaly> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_opt(
                "UPDATE anomaly
                    SET dismissed_at = COALESCE(dismissed_at, NOW()),
                        dismissed_reason = COALESCE($3, dismissed_reason)
                  WHERE id = $1 AND user_id = $2
                  RETURNING *",
                &[&id, &user_id, &reason],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or_else(|| ApplicationError::NotFound("anomaly not found".into()))?;
        Ok(row_to_anomaly(&row))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.operation = "anomaly.undismiss"))]
    async fn undismiss(&self, user_id: Uuid, id: Uuid) -> Result<Anomaly> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_opt(
                "UPDATE anomaly
                    SET dismissed_at = NULL, dismissed_reason = NULL
                  WHERE id = $1 AND user_id = $2
                  RETURNING *",
                &[&id, &user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or_else(|| ApplicationError::NotFound("anomaly not found".into()))?;
        Ok(row_to_anomaly(&row))
    }
}
