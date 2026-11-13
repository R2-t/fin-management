use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError, application::currency::minor_unit, auth::AuthPrincipal,
    model::ledger::WalletBalance, AppState,
};

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub wallet_id: Uuid,
    pub currency: String,
    /// Decimal places for `currency`, so clients render minor units correctly.
    pub currency_minor_unit: u8,
    /// Signed. Negative means the wallet is overdrawn.
    pub balance_minor: i64,
    pub entry_count: i64,
}

impl From<WalletBalance> for ResponseBody {
    fn from(b: WalletBalance) -> Self {
        Self {
            wallet_id: b.wallet_id,
            currency_minor_unit: minor_unit(&b.currency),
            currency: b.currency,
            balance_minor: b.balance_minor,
            entry_count: b.entry_count,
        }
    }
}

/// A Wallet's balance: the sum of its ledger entries (ADR-0004).
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(wallet_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let balance = state
        .ledger_service
        .balance(principal.user_id, wallet_id)
        .await?;

    Ok((StatusCode::OK, Json(ResponseBody::from(balance))).into_response())
}
