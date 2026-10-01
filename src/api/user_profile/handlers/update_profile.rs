use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use std::sync::Arc;

use super::get_profile::ResponseBody;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// Partial update: omitted fields keep their current value.
#[derive(Debug, Deserialize)]
pub struct RequestBody {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub base_currency: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let profile = state
        .user_profile_service
        .update(
            principal.user_id,
            body.timezone,
            body.base_currency,
            body.locale,
        )
        .await?;

    Ok((StatusCode::OK, Json(ResponseBody::from(profile))).into_response())
}
