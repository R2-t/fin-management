use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// Projects whether a budget will go over its limit before the period ends.
///
/// The forecast is persisted with its assumptions, so it can be compared and
/// back-tested later.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(budget_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let timezone = state.user_profile_service.get(principal.user_id).await?.timezone;
    let today = crate::application::user_profile::service::today_in(&timezone)?;
    let forecast = state
        .forecast_service
        .overspend(principal.user_id, budget_id, today, &timezone)
        .await?;

    Ok((StatusCode::OK, Json(forecast)).into_response())
}
