use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Serialize;
use std::sync::Arc;

use crate::{api::ApiError, auth::AuthPrincipal, model::user_profile::UserProfile, AppState};

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub user_id: uuid::Uuid,
    pub timezone: String,
    pub base_currency: String,
    pub locale: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<UserProfile> for ResponseBody {
    fn from(p: UserProfile) -> Self {
        Self {
            user_id: p.user_id,
            timezone: p.timezone,
            base_currency: p.base_currency,
            locale: p.locale,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
) -> Result<impl IntoResponse, ApiError> {
    let profile = state.user_profile_service.get(principal.user_id).await?;
    Ok((StatusCode::OK, Json(ResponseBody::from(profile))).into_response())
}
