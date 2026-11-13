use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{ForecastRepository, Result};
use crate::model::forecast::{Forecast, ForecastKind};
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgForecastRepo {
    pool: PgPool,
}

impl PgForecastRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_forecast(row: &tokio_postgres::Row) -> Forecast {
    Forecast {
        id: row.get("id"),
        user_id: row.get("user_id"),
        wallet_id: row.get("wallet_id"),
        kind: ForecastKind::parse(row.get("kind")).unwrap_or(ForecastKind::CashRunway),
        generated_at: row.get("generated_at"),
        horizon_days: row.get("horizon_days"),
        result: row.get("result"),
        assumptions: row.get("assumptions"),
        model_version: row.get("model_version"),
    }
}

impl ForecastRepository for PgForecastRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "forecast.insert"))]
    async fn insert(&self, forecast: Forecast) -> Result<Forecast> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO forecast
                    (id, user_id, wallet_id, kind, generated_at, horizon_days, result, assumptions, model_version)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                   RETURNING *"#,
                &[
                    &forecast.id,
                    &forecast.user_id,
                    &forecast.wallet_id,
                    &forecast.kind.as_str(),
                    &forecast.generated_at,
                    &forecast.horizon_days,
                    &forecast.result,
                    &forecast.assumptions,
                    &forecast.model_version,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_forecast(&row))
    }

    /// The user's forecasts, newest first.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "forecast.list"))]
    async fn list(
        &self,
        user_id: Uuid,
        kind: Option<ForecastKind>,
    ) -> Result<Vec<Forecast>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                "SELECT * FROM forecast
                  WHERE user_id = $1 AND ($2::text IS NULL OR kind = $2)
                  ORDER BY generated_at DESC
                  LIMIT 100",
                &[&user_id, &kind.map(|k| k.as_str())],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_forecast).collect())
    }
}
