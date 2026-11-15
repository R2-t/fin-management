use axum::{extract::State, response::IntoResponse, Extension, Json};
use std::sync::Arc;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
) -> Result<impl IntoResponse, ApiError> {
    let groups = state.account_group_service.list(principal.user_id).await?;
    Ok(Json(groups))
}
