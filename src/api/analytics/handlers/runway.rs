use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// Projects how long a Wallet's money lasts: the trough it reaches and the
/// date the balance recovers.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(wallet_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let rules = state.recurring_rule_service.list(principal.user_id).await?;
    let incomes = state.income_service.list(principal.user_id).await?;

    let runway = state
        .runway_service
        .runway(principal.user_id, wallet_id, &rules, &incomes)
        .await?;

    Ok((StatusCode::OK, Json(runway)).into_response())
}
