use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::{Result, UserProfileRepository};
use crate::model::user_profile::UserProfile;
use crate::repository::{PgPool, RepositoryError};

pub struct PgUserProfileRepo {
    pool: PgPool,
}

impl PgUserProfileRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_profile(row: &tokio_postgres::Row) -> UserProfile {
    UserProfile {
        user_id: row.get("user_id"),
        timezone: row.get("timezone"),
        base_currency: row.get("base_currency"),
        locale: row.get("locale"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

impl UserProfileRepository for PgUserProfileRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "user_profile.get"))]
    async fn get(&self, user_id: Uuid) -> Result<Option<UserProfile>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt("SELECT * FROM user_profile WHERE user_id = $1", &[&user_id])
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row.as_ref().map(row_to_profile))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "user_profile.upsert"))]
    async fn upsert(
        &self,
        user_id: Uuid,
        timezone: &str,
        base_currency: &str,
        locale: Option<&str>,
    ) -> Result<UserProfile> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO user_profile (user_id, timezone, base_currency, locale)
                   VALUES ($1, $2, $3, $4)
                   ON CONFLICT (user_id) DO UPDATE SET
                       timezone      = EXCLUDED.timezone,
                       base_currency = EXCLUDED.base_currency,
                       locale        = EXCLUDED.locale,
                       updated_at    = NOW()
                   RETURNING *"#,
                &[&user_id, &timezone, &base_currency, &locale],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_profile(&row))
    }
}
