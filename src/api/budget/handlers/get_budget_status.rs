use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// The budget's current period and how it is going.
///
/// The period is materialised on first ask, with the effective limit the carry
/// rules give it.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let today = chrono::Utc::now().date_naive();
    let status = state
        .budget_service
        .current_status(principal.user_id, id, today)
        .await?;

    Ok((StatusCode::OK, Json(status)).into_response())
}
