use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    /// When the money arrived; defaults to now.
    #[serde(default)]
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: Uuid,
    pub income_schedule_id: Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub amount_minor: i64,
    pub currency: String,
    /// True when this call found the arrival already recorded. The receipt and
    /// the wallet balance are the same either way; the flag is there so a
    /// double-click is visible rather than silent.
    pub repeat: bool,
}

/// Records that the expected income arrived. The expectation becomes money and
/// the ledger is credited in the same transaction (ADR-0008).
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let timezone = state.user_profile_service.get(principal.user_id).await?.timezone;
    let receipt = state
        .income_service
        .receive(principal.user_id, id, body.occurred_at, &timezone)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(ResponseBody {
            id: receipt.id,
            income_schedule_id: receipt.income_schedule_id,
            occurred_at: receipt.occurred_at,
            amount_minor: receipt.amount_minor,
            currency: receipt.currency,
            repeat: receipt.repeat,
        }),
    )
        .into_response())
}
