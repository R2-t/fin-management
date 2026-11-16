use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    state.account_group_service.delete(principal.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
