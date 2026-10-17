use tracing::instrument;
use crate::application::inbound::repositories::{
    CachedRate, RateCacheRepository, RateHistoryReader, RateHistoryWriter, Result,
};
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgRateCacheRepo {
    pool: PgPool,
}

impl PgRateCacheRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_cached_rate(row: &tokio_postgres::Row) -> CachedRate {
    CachedRate {
        from_currency: row.get("from_currency"),
        to_currency: row.get("to_currency"),
        rate_hour: row.get("rate_hour"),
        rate_scaled: row.get("rate_scaled"),
        source: row.get("source"),
        stale: row.get("stale"),
    }
}

impl RateCacheRepository for PgRateCacheRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "rate_cache.get"))]
    async fn get(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<CachedRate>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                r#"SELECT from_currency, to_currency, rate_hour, rate_scaled, source, stale
                   FROM exchange_rate_cache
                   WHERE from_currency = $1 AND to_currency = $2 AND rate_hour = $3"#,
                &[&from_currency, &to_currency, &rate_hour],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row.map(|r| row_to_cached_rate(&r)))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "rate_cache.get_most_recent"))]
    async fn get_most_recent(
        &self,
        from_currency: &str,
        to_currency: &str,
    ) -> Result<Option<CachedRate>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                r#"SELECT from_currency, to_currency, rate_hour, rate_scaled, source, stale
                   FROM exchange_rate_cache
                   WHERE from_currency = $1 AND to_currency = $2
                   ORDER BY rate_hour DESC
                   LIMIT 1"#,
                &[&from_currency, &to_currency],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row.map(|r| row_to_cached_rate(&r)))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "rate_cache.put"))]
    async fn put(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
        rate_scaled: i64,
        source: &str,
    ) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        conn.execute(
            r#"INSERT INTO exchange_rate_cache (from_currency, to_currency, rate_hour, rate_scaled, source)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (from_currency, to_currency, rate_hour) DO NOTHING"#,
            &[&from_currency, &to_currency, &rate_hour, &rate_scaled, &source],
        )
        .await
        .map_err(RepositoryError::DbError)?;

        Ok(())
    }
}

impl RateHistoryReader for PgRateCacheRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "rate_cache.latest_history"))]
    async fn latest_history(
        &self,
        from_currency: &str,
        to_currency: &str,
    ) -> Result<Option<CachedRate>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_opt(
                r#"SELECT from_currency, to_currency, rate_hour, rate_scaled, source
                     FROM fx_rate_history
                    WHERE from_currency = $1 AND to_currency = $2
                    ORDER BY rate_hour DESC
                    LIMIT 1"#,
                &[&from_currency, &to_currency],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(row.map(|r| CachedRate {
            from_currency: r.get("from_currency"),
            to_currency: r.get("to_currency"),
            rate_hour: r.get("rate_hour"),
            rate_scaled: r.get("rate_scaled"),
            source: r.get("source"),
            stale: true,
        }))
    }
}

impl RateHistoryWriter for PgRateCacheRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "rate_cache.record_history"))]
    async fn record_history(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
        rate_scaled: i64,
        source: &str,
    ) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        conn.execute(
            r#"INSERT INTO fx_rate_history
                   (from_currency, to_currency, rate_hour, rate_scaled, source)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (from_currency, to_currency, rate_hour) DO NOTHING"#,
            &[&from_currency, &to_currency, &rate_hour, &rate_scaled, &source],
        )
        .await
        .map_err(RepositoryError::DbError)?;
        Ok(())
    }
}
