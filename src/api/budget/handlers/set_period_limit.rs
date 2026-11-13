use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    /// The first day of the period, as YYYY-MM-DD.
    pub period_start: chrono::NaiveDate,
    /// The new limit, in minor units.
    pub amount_minor: i64,
}

/// Sets a period's limit. The edit applies to that period only.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let period = state
        .budget_service
        .set_period_limit(
            principal.user_id,
            id,
            body.period_start,
            body.amount_minor,
        )
        .await?;

    Ok((StatusCode::OK, Json(period)).into_response())
}
