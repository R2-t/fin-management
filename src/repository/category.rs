use uuid::Uuid;

use crate::application::inbound::repositories::{CategoryRepository, Result};
use crate::model::category::Category;
use crate::repository::RepositoryError;

use super::PgPool;

pub struct PgCategoryRepo {
    pool: PgPool,
}

impl PgCategoryRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_category(row: &tokio_postgres::Row) -> Category {
    Category {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        user_id: row.get("user_id"),
        is_system: row.get("is_system"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

impl CategoryRepository for PgCategoryRepo {
    async fn create(
        &self,
        name: String,
        description: Option<String>,
        user_id: Option<Uuid>,
        is_system: bool,
    ) -> Result<Category> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_one(
                r#"INSERT INTO category (name, description, user_id, is_system)
                   VALUES ($1, $2, $3, $4)
                   RETURNING *"#,
                &[&name, &description, &user_id, &is_system],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row_to_category(&row))
    }

    async fn list(&self, user_id: Option<Uuid>) -> Result<Vec<Category>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                r#"SELECT * FROM category
                   WHERE is_system = TRUE OR user_id = $1
                   ORDER BY name"#,
                &[&user_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows.iter().map(row_to_category).collect())
    }

    async fn get(&self, id: i32) -> Result<Category> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt("SELECT * FROM category WHERE id = $1", &[&id])
            .await
            .map_err(RepositoryError::DbError)?
            .ok_or(RepositoryError::NotFound)?;

        Ok(row_to_category(&row))
    }
}
