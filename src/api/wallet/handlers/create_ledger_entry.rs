use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::list_ledger::ResponseBody;
use crate::{
    api::ApiError, application::currency::normalize_currency, auth::AuthPrincipal, AppState,
};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    /// Minor units. A positive amount credits the wallet.
    pub amount_minor: i64,
    /// ISO-4217 alpha code; defaults to the wallet's own currency.
    #[serde(default)]
    pub currency: Option<String>,
    /// When the movement happened; defaults to now.
    #[serde(default)]
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Records an opening balance or a manual adjustment against a Wallet.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(wallet_id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    // The wallet's currency is authoritative: a ledger entry in a different
    // currency would make the balance a sum across currencies (ADR-0002).
    let wallet = state
        .wallet_service
        .get(principal.user_id, wallet_id)
        .await?;

    let currency = match body.currency {
        Some(c) => normalize_currency(&c)?,
        None => wallet.currency.clone(),
    };
    if currency != wallet.currency {
        return Err(ApiError::BadRequest(format!(
            "wallet holds {}, so a ledger entry cannot be in {currency}",
            wallet.currency
        )));
    }

    let occurred_at = body.occurred_at.unwrap_or_else(chrono::Utc::now);

    // A positive amount opens the wallet; a negative one is an adjustment
    // that reduces the balance.
    let entry = if body.amount_minor > 0 {
        state
            .ledger_service
            .set_opening_balance(
                principal.user_id,
                wallet_id,
                body.amount_minor,
                currency,
                occurred_at,
            )
            .await?
    } else {
        state
            .ledger_service
            .adjust(
                principal.user_id,
                wallet_id,
                body.amount_minor,
                currency,
                occurred_at,
            )
            .await?
    };

    Ok((StatusCode::CREATED, Json(ResponseBody::from(entry))).into_response())
}
