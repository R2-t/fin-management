use axum::{extract::State, response::IntoResponse, Extension, Json};
use serde::{Deserialize, Serialize};

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub user_id: Option<uuid::Uuid>,
    pub is_system: Option<bool>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<crate::model::category::Category> for ResponseBody {
    fn from(c: crate::model::category::Category) -> Self {
        Self {
            id: c.id,
            name: c.name,
            description: c.description,
            user_id: c.user_id,
            is_system: c.is_system,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

pub async fn handler(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    // Categories created through the API are always owned by the
    // authenticated user; system categories are seeded out of band.
    let category = state
        .category_service
        .create(body.name, body.description, Some(principal.user_id), false)
        .await?;

    Ok((axum::http::StatusCode::CREATED, Json(ResponseBody::from(category))).into_response())
}