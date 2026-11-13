use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    /// Restrict the pace to one wallet. Omit for all the user's wallets.
    #[serde(default)]
    pub wallet_id: Option<uuid::Uuid>,
}

/// The spending pace a forecast assumes.
///
/// The pace covers discretionary spending only: the recorded payments of the
/// declared recurring rules are excluded, so a subscription is not subtracted
/// twice.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let rules = state.recurring_rule_service.list(principal.user_id).await?;

    let burn = state
        .burn_rate_service
        .burn_rate(principal.user_id, params.wallet_id, &rules)
        .await?;

    Ok((StatusCode::OK, Json(burn)).into_response())
}
