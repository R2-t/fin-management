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
    let timezone = state.user_profile_service.get(principal.user_id).await?.timezone;

    let runway = state
        .runway_service
        .runway(principal.user_id, wallet_id, &rules, &incomes, &timezone)
        .await?;

    // The bands, when the sidecar answered, are persisted like every other
    // forecast so they can be back-tested against the deterministic answer
    // (6.7).
    if let Some(bands) = &runway.bands {
        let result = serde_json::json!({
            "bands": bands,
            "trough_minor": runway.trough_minor,
            "recovery_date": runway.recovery_date,
            "projected_balance_minor": runway.projected_balance_minor,
        });
        let assumptions = serde_json::to_value(&runway.assumptions)
            .unwrap_or(serde_json::Value::Null);
        if let Some(version) = &runway.bands_model_version {
            state
                .forecast_service
                .persist(
                    principal.user_id,
                    crate::application::forecast::service::PersistForecast {
                        wallet_id,
                        kind: crate::model::forecast::ForecastKind::RunwayBands,
                        horizon_days: runway.horizon_days,
                        result,
                        assumptions,
                        model_version: version.clone(),
                    },
                )
                .await?;
        }
    }

    Ok((StatusCode::OK, Json(runway)).into_response())
}
