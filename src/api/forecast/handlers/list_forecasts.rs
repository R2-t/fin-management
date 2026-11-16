use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::{api::ApiError, auth::AuthPrincipal, model::forecast::ForecastKind, AppState};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    /// 'budget_overspend' | 'cash_runway'. Omit for all kinds.
    #[serde(default)]
    pub kind: Option<String>,
}

/// The user's generated forecasts, newest first.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let kind = match params.kind.as_deref() {
        None => None,
        Some(raw) => Some(ForecastKind::parse(raw).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "unknown forecast kind {raw:?}: expected budget_overspend or cash_runway"
            ))
        })?),
    };

    let forecasts = state.forecast_service.list(principal.user_id, kind).await?;
    Ok((StatusCode::OK, Json(forecasts)).into_response())
}
