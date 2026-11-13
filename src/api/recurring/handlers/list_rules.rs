use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;

use super::create_rule::ResponseBody;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// The user's recurring commitments.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
) -> Result<impl IntoResponse, ApiError> {
    let rules = state.recurring_rule_service.list(principal.user_id).await?;
    let body: Vec<ResponseBody> = rules.into_iter().map(ResponseBody::from).collect();
    Ok((StatusCode::OK, Json(body)).into_response())
}
