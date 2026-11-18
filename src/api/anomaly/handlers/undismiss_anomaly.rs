use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// "No, I was wrong" happens; without this the user's only recourse is a
/// database edit.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .anomaly_service
        .undismiss(principal.user_id, id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
