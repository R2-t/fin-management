use tracing::instrument;

use crate::application::{error::ApplicationError, inbound::repositories::CategoryRepository};
use crate::model::category::Category;

pub struct CategoryService<C>
where
    C: CategoryRepository,
{
    categories: C,
}

impl<C> CategoryService<C>
where
    C: CategoryRepository,
{
    pub fn new(categories: C) -> Self {
        Self { categories }
    }

    #[instrument(skip(self))]
    pub async fn create(
        &self,
        name: String,
        description: Option<String>,
        user_id: Option<uuid::Uuid>,
        is_system: bool,
    ) -> Result<Category, ApplicationError> {
        if name.trim().is_empty() {
            return Err(ApplicationError::Validation("category name cannot be empty".into()));
        }
        self.categories.create(name, description, user_id, is_system).await
    }

    #[instrument(skip(self))]
    pub async fn list(
        &self,
        user_id: Option<uuid::Uuid>,
    ) -> Result<Vec<Category>, ApplicationError> {
        self.categories.list(user_id).await
    }

    #[instrument(skip(self))]
    pub async fn get(&self, id: i32) -> Result<Category, ApplicationError> {
        self.categories.get(id).await
    }
}
