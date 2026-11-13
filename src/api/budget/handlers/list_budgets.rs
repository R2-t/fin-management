use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;

use super::create_budget::ResponseBody;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

/// The user's budgets.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
) -> Result<impl IntoResponse, ApiError> {
    let budgets = state.budget_service.list(principal.user_id).await?;

    let mut body = Vec::with_capacity(budgets.len());
    for budget in budgets {
        let wallet = state
            .wallet_service
            .get(principal.user_id, budget.wallet_id)
            .await?;
        let mut response = ResponseBody::from(budget);
        response.currency_minor_unit = crate::application::currency::minor_unit(&wallet.currency);
        response.currency = wallet.currency;
        body.push(response);
    }

    Ok((StatusCode::OK, Json(body)).into_response())
}
