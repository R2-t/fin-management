use crate::application::inbound::repositories::{CachedRate, RateCacheRepository, Result};
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
        rate: row.get("rate"),
        source: row.get("source"),
        stale: row.get("stale"),
    }
}

impl RateCacheRepository for PgRateCacheRepo {
    async fn get(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<CachedRate>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                r#"SELECT from_currency, to_currency, rate_hour, rate::float8 AS rate, source, stale
                   FROM exchange_rate_cache
                   WHERE from_currency = $1 AND to_currency = $2 AND rate_hour = $3"#,
                &[&from_currency, &to_currency, &rate_hour],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row.map(|r| row_to_cached_rate(&r)))
    }

    async fn get_most_recent(
        &self,
        from_currency: &str,
        to_currency: &str,
    ) -> Result<Option<CachedRate>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                r#"SELECT from_currency, to_currency, rate_hour, rate::float8 AS rate, source, stale
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

    async fn put(
        &self,
        from_currency: &str,
        to_currency: &str,
        rate_hour: chrono::DateTime<chrono::Utc>,
        rate: f64,
        source: &str,
    ) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        conn.execute(
            r#"INSERT INTO exchange_rate_cache (from_currency, to_currency, rate_hour, rate, source)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (from_currency, to_currency, rate_hour) DO NOTHING"#,
            &[&from_currency, &to_currency, &rate_hour, &rate, &source],
        )
        .await
        .map_err(RepositoryError::DbError)?;

        Ok(())
    }
}
