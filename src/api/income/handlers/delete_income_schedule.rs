use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// Deletes an income schedule. Receipts already recorded keep their ledger
/// entries: the money arrived, and deleting the expectation does not un-arrive
/// it.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    state.income_service.delete(principal.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
