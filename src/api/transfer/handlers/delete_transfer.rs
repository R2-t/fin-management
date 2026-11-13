use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use std::sync::Arc;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// Deletes a transfer and reverses its ledger legs.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<uuid::Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    state.transfer_service.delete(principal.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
