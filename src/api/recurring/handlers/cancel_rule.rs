use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;
use uuid::Uuid;

use super::create_rule::ResponseBody;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// Cancels a rule. It stops being forecast but keeps its history: the user may
/// cancel a subscription and later restart it.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let rule = state.recurring_rule_service.cancel(principal.user_id, id).await?;
    Ok((StatusCode::OK, Json(ResponseBody::from(rule))).into_response())
}
